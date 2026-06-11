#!/usr/bin/env bash
# Generic dev runner for fkst packages.
#
#   scripts/run.sh test [package]
#       Run fkst-framework --self-test once, then conformance + test for flat
#       packages. Composed packages skip single-package conformance and still
#       run tests. Full test also runs composed graph conformance. This is the
#       single CI and local test entrypoint.
#
#   scripts/run.sh check
#       Run hermetic repository checks only. Does not resolve or execute BIN.
#
#   scripts/run.sh test-composed
#       Run only composed graph conformance for packages with composed.deps.
#
#   scripts/run.sh run <package> <department> [event-json]
#   scripts/run.sh run <package> <department> --event-file <path>
#       One-shot run a department against the REAL host environment via
#       `fkst-framework run`: decode emitted RAISED events and dump the <RT>
#       scratch tree. Generic across packages; pass package-specific config via
#       env, e.g.:
#         PACKAGE_CONFIG=value scripts/run.sh run example-package example_department
#       Reuses $FKST_RUNTIME_ROOT if already set (run twice with the same one to
#       observe dedup), else uses a fresh temp dir. Never sets FKST_GITHUB_WRITE,
#       so a read-only inbound dogfood stays read-only.
#
#   scripts/run.sh supervise <package>
#       Start the real fkst-framework supervise event loop for one package.
#       Uses fresh temporary FKST_RUNTIME_ROOT and FKST_DURABLE_ROOT directories
#       and runs in the foreground until Ctrl-C. FKST_PROJECT_ROOT can override
#       the default project root of packages/<package>.
#
#   scripts/run.sh build
#       Local-only helper: update the fkst-substrate dev checkout and build
#       fkst-framework. test/run/supervise ensure a traceable local BIN is built
#       from the current fkst-substrate working tree before running.
#
# fkst-framework binary resolution (priority): $BIN > repo .env `BIN=` > PATH >
# sibling ../fkst-substrate/target/debug/fkst-framework.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

resolve_bin() {
  if [ -z "${BIN:-}" ] && [ -f "$ROOT/.env" ]; then
    # `|| true`: no BIN= line is fine under set -o pipefail. Strip optional
    # surrounding quotes and a trailing ` # comment`.
    BIN="$(grep -E '^BIN=' "$ROOT/.env" 2>/dev/null | tail -1 | cut -d= -f2- || true)"
    BIN="${BIN%%[[:space:]]#*}"
    BIN="${BIN%\"}"; BIN="${BIN#\"}"; BIN="${BIN%\'}"; BIN="${BIN#\'}"
  fi
  if [ -z "${BIN:-}" ]; then
    if command -v fkst-framework >/dev/null 2>&1; then
      BIN="$(command -v fkst-framework)"
    elif [ -x "$ROOT/../fkst-substrate/target/debug/fkst-framework" ]; then
      BIN="$ROOT/../fkst-substrate/target/debug/fkst-framework"
    fi
  fi
  if [ -z "${BIN:-}" ] || [ ! -x "$BIN" ]; then
    if [ -n "${CI:-}" ] || [ -n "${GITHUB_ACTIONS:-}" ]; then
      echo "error: fkst-framework binary is not executable in CI: ${BIN:-<unset>}" >&2
      echo "  CI must build fkst-substrate and inject BIN; scripts/run.sh will not build in CI." >&2
      exit 1
    fi
    echo "error: fkst-framework binary not found (\$BIN, .env, PATH, ../fkst-substrate)." >&2
    echo "  fix: cp env.example .env (set BIN=), or build the engine:" >&2
    echo "       scripts/run.sh build" >&2
    exit 1
  fi
  export BIN
}

# Resolve a path to its physical location, following file symlinks too (portable:
# no realpath / `readlink -f` dependency, works with macOS BSD readlink). This
# lets a symlinked BIN (e.g. a PATH install pointing into a checkout target) be
# traced back to its fkst-substrate checkout.
resolve_phys_path() {
  local p="$1" target dir
  while [ -L "$p" ]; do
    target="$(readlink "$p")" || break
    case "$target" in
      /*) p="$target" ;;
      *)  p="$(cd "$(dirname "$p")" 2>/dev/null && pwd -P)/$target" ;;
    esac
  done
  dir="$(cd "$(dirname "$p")" 2>/dev/null && pwd -P)" || return 1
  printf '%s/%s\n' "$dir" "$(basename "$p")"
}

ensure_fresh_bin() {
  if [ -n "${CI:-}" ] || [ -n "${GITHUB_ACTIONS:-}" ]; then
    return 0
  fi
  if [ -n "${FKST_NO_AUTOBUILD:-}" ]; then
    echo "warning: FKST_NO_AUTOBUILD set; skipping fkst-framework freshness build" >&2
    return 0
  fi

  local phys substrate suffix
  suffix="/target/debug/fkst-framework"
  phys="$(resolve_phys_path "$BIN")" || phys="$BIN"

  if [[ "$phys" == *"$suffix" ]]; then
    substrate="${phys%"$suffix"}"
  else
    substrate=""
  fi
  if [ -z "$substrate" ] || [ ! -d "$substrate/.git" ] || [ ! -f "$substrate/Cargo.toml" ]; then
    echo "warning: cannot trace BIN to an fkst-substrate checkout; skipping freshness build: $BIN" >&2
    return 0
  fi

  echo "ensuring fkst-framework is built from current source: $substrate" >&2
  if ! cargo build --manifest-path "$substrate/Cargo.toml" -p fkst-framework 1>&2; then
    echo "error: fkst-framework freshness build failed; refusing to continue with a potentially stale BIN" >&2
    exit 1
  fi
}

usage() {
  sed -n '2,32p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'
}

cmd_check() {
  python3 "$ROOT/scripts/check_repo.py"
  python3 "$ROOT/scripts/check_repo_test.py"
  python3 "$ROOT/scripts/bin_cache_test.py"
}

check_test_file_coverage() {
  local report_dir="$1" expected actual missing
  expected="$(mktemp "${TMPDIR:-/tmp}/fkst-test-files-expected.XXXXXX")"
  actual="$(mktemp "${TMPDIR:-/tmp}/fkst-test-files-actual.XXXXXX")"
  missing="$(mktemp "${TMPDIR:-/tmp}/fkst-test-files-missing.XXXXXX")"

  (
    cd "$ROOT"
    find packages \( -path '*/tests/*_test.lua' -o -path '*/departments/*/*_test.lua' \) -type f -print | LC_ALL=C sort -u
  ) > "$expected"

  python3 - "$report_dir" <<'PY' | LC_ALL=C sort -u > "$actual"
import json
import sys
from pathlib import Path

report_dir = Path(sys.argv[1])
for report_path in sorted(report_dir.glob("*.json")):
    with report_path.open(encoding="utf-8") as handle:
        report = json.load(handle)
    if report.get("schema") != "fkst.test.report.v1":
        raise SystemExit(f"bad test report schema in {report_path}: {report.get('schema')!r}")
    summary = report.get("summary")
    if not isinstance(summary, dict):
        raise SystemExit(f"missing test report summary in {report_path}")
    if int(summary.get("failed", 0)) != 0:
        raise SystemExit(f"test report contains failures in {report_path}")
    for test in report.get("tests", []):
        if not isinstance(test, dict) or test.get("status") != "pass":
            continue
        owner = test.get("owner_namespace")
        file_name = test.get("file")
        if not isinstance(owner, str) or not isinstance(file_name, str):
            continue
        if not (file_name.startswith("tests/") or file_name.startswith("departments/")) or not file_name.endswith("_test.lua"):
            continue
        print(f"packages/{owner}/{file_name}")
PY

  comm -23 "$expected" "$actual" > "$missing"
  if [ -s "$missing" ]; then
    echo "error: G5 engine test coverage failed; these *_test.lua files produced zero report-json pass results:" >&2
    sed 's/^/  /' "$missing" >&2
    echo "  Each *_test.lua must contribute at least one real engine-enumerated top-level test." >&2
    rm -f "$expected" "$actual" "$missing"
    return 1
  fi

  rm -f "$expected" "$actual" "$missing"
  echo "OK: G5 every *_test.lua produced an engine report-json pass"
}

check_sdk_primitives() {
  local probe_dir report_file
  probe_dir="$(mktemp -d "${TMPDIR:-/tmp}/fkst-sdk-probe.XXXXXX")"
  mkdir -p "$probe_dir/tests"
  printf 'return {}\n' > "$probe_dir/core.lua"
  cat > "$probe_dir/tests/sdk_primitives_test.lua" <<'LUA'
local t = fkst.test

local function cjk_char()
  return string.char(0xe6, 0xb5, 0x8b)
end

local function emoji_char()
  return string.char(0xf0, 0x9f, 0x98, 0x80)
end

local function assert_valid_utf8(value)
  local ok, len = pcall(utf8.len, tostring(value or ""))
  t.is_true(ok and len ~= nil)
end

return {
  test_truncate_utf8_sdk_primitive_is_deployed = function()
    t.eq(type(truncate_utf8), "function")
    local cjk = cjk_char()
    local emoji = emoji_char()
    local mixed = "ab" .. cjk .. "cd"

    t.eq(truncate_utf8(mixed, 2), "ab")
    t.eq(truncate_utf8(mixed, 3), "ab")
    t.eq(truncate_utf8(mixed, 4), "ab")
    t.eq(truncate_utf8(mixed, 5), "ab" .. cjk)
    t.eq(truncate_utf8(mixed, 6), "ab" .. cjk .. "c")
    t.eq(truncate_utf8("", 3), "")
    t.eq(truncate_utf8(cjk, 2), "")
    t.eq(truncate_utf8(emoji .. "x", 3), "")
    t.eq(truncate_utf8("ab" .. emoji .. "x", 6), "ab" .. emoji)
    assert_valid_utf8(truncate_utf8(mixed, 1))
    assert_valid_utf8(truncate_utf8(mixed, 7))
    assert_valid_utf8(truncate_utf8("ab" .. emoji .. "x", 5))
    assert_valid_utf8(truncate_utf8("ab" .. emoji .. "x", 6))
  end,
}
LUA

  report_file="$probe_dir/report.json"
  if ! "$BIN" test --project-root "$probe_dir" --package-root "$probe_dir" --report-json "$report_file"; then
    rm -rf "$probe_dir"
    echo "error: required SDK primitive is unavailable or invalid: truncate_utf8(s, max_bytes)" >&2
    return 1
  fi
  rm -rf "$probe_dir"
  echo "OK: SDK primitive truncate_utf8 is available in BIN"
}

cmd_test() {
  local target="${1:-}" ran=0 fail=0 pkg name
  local self_rt report_dir report_file

  report_dir="$(mktemp -d "${TMPDIR:-/tmp}/fkst-test-reports.XXXXXX")"

  echo "=== self-test ==="
  if [ -n "${FKST_RUNTIME_ROOT:-}" ]; then
    if ! "$BIN" --self-test; then
      fail=$((fail + 1))
    fi
  else
    self_rt="$(mktemp -d "${TMPDIR:-/tmp}/fkst-self-test.XXXXXX")"
    if ! FKST_RUNTIME_ROOT="$self_rt" "$BIN" --self-test; then
      fail=$((fail + 1))
    fi
  fi

  echo "=== sdk-primitives ==="
  if ! check_sdk_primitives; then
    fail=$((fail + 1))
  fi

  for pkg in "$ROOT"/packages/*/; do
    [ -d "$pkg" ] || continue
    name="$(basename "$pkg")"
    if [ -n "$target" ] && [ "$name" != "$target" ]; then continue; fi
    echo "=== $name ==="
    ran=$((ran + 1))
    if [ -f "$pkg/composed.deps" ]; then
      echo "skip single-package conformance for composed package: $name"
    else
      if ! "$BIN" conformance --project-root "$pkg" --package-root "$pkg"; then
        fail=$((fail + 1))
        continue
      fi
    fi
    report_file="$report_dir/$name.json"
    if ! "$BIN" test --project-root "$pkg" --package-root "$pkg" --report-json "$report_file"; then
      fail=$((fail + 1))
    fi
  done
  if [ "$ran" -eq 0 ]; then
    if [ -n "$target" ]; then
      echo "no packages matched for '$target'" >&2
    else
      echo "no packages matched" >&2
    fi
    exit 1
  fi
  if [ -z "$target" ]; then
    if ! cmd_test_composed; then
      fail=$((fail + 1))
    fi
    if [ "$fail" -eq 0 ]; then
      if ! check_test_file_coverage "$report_dir"; then
        fail=$((fail + 1))
      fi
    fi
  fi
  if [ "$fail" -ne 0 ]; then
    rm -rf "$report_dir"
    echo "FAILED: $fail failure(s) across $ran package(s)" >&2; exit 1
  fi
  rm -rf "$report_dir"
  echo "OK: $ran package(s)"
}

collect_composed_package() {
  local name="$1" pkg dep
  pkg="$ROOT/packages/$name"
  [ -d "$pkg" ] || { echo "error: composed package dependency not found: $name" >&2; return 1; }
  case " ${COMPOSED_SEEN[*]-} " in
    *" $name "*) return 0 ;;
  esac
  COMPOSED_SEEN+=("$name")
  if [ -f "$pkg/composed.deps" ]; then
    while IFS= read -r dep || [ -n "$dep" ]; do
      dep="${dep%%#*}"
      dep="${dep#"${dep%%[![:space:]]*}"}"
      dep="${dep%"${dep##*[![:space:]]}"}"
      [ -n "$dep" ] || continue
      collect_composed_package "$dep" || return 1
    done < "$pkg/composed.deps"
  fi
}

cmd_test_composed() {
  local pkg name args
  COMPOSED_SEEN=()
  for pkg in "$ROOT"/packages/*/; do
    [ -d "$pkg" ] || continue
    [ -f "$pkg/composed.deps" ] || continue
    name="$(basename "$pkg")"
    collect_composed_package "$name" || return 1
  done
  if [ "${#COMPOSED_SEEN[@]}" -eq 0 ]; then
    echo "no composed packages matched"
    return 0
  fi

  args=()
  for name in "${COMPOSED_SEEN[@]}"; do
    args+=(--package-root "$ROOT/packages/$name")
  done
  echo "=== composed conformance ==="
  "$BIN" conformance --project-root "$ROOT" "${args[@]}"
}

cmd_run() {
  local pkg="${1:-}" dept="${2:-}"
  if [ -z "$pkg" ] || [ -z "$dept" ]; then
    echo "usage: scripts/run.sh run <package> <department> [event-json]" >&2
    echo "   or: scripts/run.sh run <package> <department> --event-file <path>" >&2
    exit 1
  fi
  shift 2

  local event="{\"payload\":{}}" event_file="" inline_event=""
  while [ "$#" -gt 0 ]; do
    case "$1" in
      --event-file)
        if [ -n "$event_file" ]; then
          echo "error: --event-file can only be provided once" >&2
          exit 1
        fi
        if [ "$#" -lt 2 ] || [ -z "${2:-}" ]; then
          echo "error: --event-file requires a readable path" >&2
          exit 1
        fi
        event_file="$2"
        shift 2
        ;;
      --event-file=*)
        if [ -n "$event_file" ]; then
          echo "error: --event-file can only be provided once" >&2
          exit 1
        fi
        event_file="${1#--event-file=}"
        if [ -z "$event_file" ]; then
          echo "error: --event-file requires a readable path" >&2
          exit 1
        fi
        shift
        ;;
      --*)
        echo "error: unknown run option: $1" >&2
        exit 1
        ;;
      *)
        if [ -n "$inline_event" ]; then
          echo "error: run accepts only one inline event JSON argument" >&2
          exit 1
        fi
        inline_event="$1"
        shift
        ;;
    esac
  done

  if [ -n "$event_file" ] && [ -n "$inline_event" ]; then
    echo "error: use either inline event JSON or --event-file, not both" >&2
    exit 1
  fi
  if [ -n "$event_file" ]; then
    [ -f "$event_file" ] || { echo "error: event file does not exist: $event_file" >&2; exit 1; }
    [ -r "$event_file" ] || { echo "error: event file is not readable: $event_file" >&2; exit 1; }
    event="$(< "$event_file")"
  elif [ -n "$inline_event" ]; then
    event="$inline_event"
  fi

  local pkgdir="$ROOT/packages/$pkg" lua
  lua="$pkgdir/departments/$dept/main.lua"
  [ -f "$lua" ] || { echo "error: no department at $lua" >&2; exit 1; }

  local rt fresh=0
  if [ -n "${FKST_RUNTIME_ROOT:-}" ]; then
    rt="$FKST_RUNTIME_ROOT"
  else
    rt="$(mktemp -d "${TMPDIR:-/tmp}/fkst-run.XXXXXX")"; fresh=1
  fi
  export FKST_RUNTIME_ROOT="$rt"

  echo "BIN=$BIN"
  echo "run $pkg/$dept  FKST_RUNTIME_ROOT=$rt${fresh:+ (fresh)}"
  if [ -n "${FKST_GITHUB_REPO:-}" ]; then echo "FKST_GITHUB_REPO=$FKST_GITHUB_REPO"; fi

  # Capture rc without set -e aborting at the assignment, so failure logs and
  # any partial RAISED/<RT> still print; propagate rc as the run's exit.
  local out rc=0
  out="$("$BIN" run "$lua" --project-root "$ROOT" --package-root "$pkgdir" --event "$event" 2>&1)" || rc=$?

  echo "--- logs ---"
  printf '%s\n' "$out" | grep -vE '^RAISED:' || true
  echo "--- raised events (decoded) ---"
  local b64
  b64="$(printf '%s\n' "$out" | grep '^RAISED:' | sed 's/^RAISED: //' | tail -1 || true)"
  if [ -n "$b64" ]; then
    printf '%s' "$b64" | base64 -d 2>/dev/null | python3 -m json.tool 2>/dev/null \
      || { echo "(raw)"; printf '%s' "$b64" | base64 -d 2>/dev/null; }
  else
    echo "  (no events raised)"
  fi
  echo "--- <RT> tree ---"
  find "$rt" -type f 2>/dev/null | sort | while read -r f; do
    echo "  ${f#"$rt"/} = $(cat "$f" 2>/dev/null | head -c 120)"
  done
  [ "$rc" -eq 0 ] || echo "--- run exited $rc ---" >&2
  return "$rc"
}

cmd_supervise() {
  local pkg="${1:-}"
  if [ -z "$pkg" ]; then
    echo "usage: scripts/run.sh supervise <package>" >&2; exit 1
  fi
  local pkgdir="$ROOT/packages/$pkg"
  [ -d "$pkgdir" ] || { echo "error: no package at $pkgdir" >&2; exit 1; }

  local project_root rt durable
  project_root="${FKST_PROJECT_ROOT:-$pkgdir}"
  rt="$(mktemp -d "${TMPDIR:-/tmp}/fkst-supervise-rt.XXXXXX")"
  durable="$(mktemp -d "${TMPDIR:-/tmp}/fkst-supervise-durable.XXXXXX")"
  if [ "$rt" = "$durable" ]; then
    echo "error: FKST_RUNTIME_ROOT and FKST_DURABLE_ROOT resolved to the same directory" >&2
    exit 1
  fi
  export FKST_RUNTIME_ROOT="$rt"
  export FKST_DURABLE_ROOT="$durable"

  echo "BIN=$BIN"
  echo "FKST_RUNTIME_ROOT=$FKST_RUNTIME_ROOT"
  echo "FKST_DURABLE_ROOT=$FKST_DURABLE_ROOT"
  echo "This starts the real supervise event loop in the foreground. Press Ctrl-C to stop."
  echo "exec: \"$BIN\" supervise --project-root \"$project_root\" --package-root \"$pkgdir\" --framework-bin \"$BIN\""
  exec "$BIN" supervise --project-root "$project_root" --package-root "$pkgdir" --framework-bin "$BIN"
}

cmd_build() {
  local substrate="${FKST_SUBSTRATE:-}"
  if [ -z "$substrate" ]; then
    if [ -d "/Users/auric/fkst-substrate/.git" ]; then
      substrate="/Users/auric/fkst-substrate"
    elif [ -d "$ROOT/../fkst-substrate/.git" ]; then
      substrate="$ROOT/../fkst-substrate"
    fi
  fi
  if [ -z "$substrate" ] || [ ! -d "$substrate/.git" ]; then
    echo "error: fkst-substrate checkout not found (set FKST_SUBSTRATE, use /Users/auric/fkst-substrate, or sibling ../fkst-substrate)." >&2
    exit 1
  fi

  local branch
  branch="$(git -C "$substrate" branch --show-current)"
  if [ "$branch" != "dev" ]; then
    echo "error: refusing to build from $substrate on branch '$branch'; switch to dev first." >&2
    exit 1
  fi

  git -C "$substrate" pull
  cargo build --manifest-path "$substrate/Cargo.toml" -p fkst-framework
  echo "OK: built $substrate/target/debug/fkst-framework"
}

case "${1:-}" in
  check) shift; cmd_check "$@" ;;
  test) shift; cmd_check; resolve_bin; ensure_fresh_bin; cmd_test "$@" ;;
  test-composed) shift; cmd_check; resolve_bin; ensure_fresh_bin; cmd_test_composed "$@" ;;
  run)  shift; resolve_bin; ensure_fresh_bin; cmd_run "$@" ;;
  supervise) shift; resolve_bin; ensure_fresh_bin; cmd_supervise "$@" ;;
  build) shift; cmd_build "$@" ;;
  -h|--help|help|"") usage ;;
  *) echo "unknown subcommand: $1" >&2; usage; exit 1 ;;
esac
