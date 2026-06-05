#!/usr/bin/env bash
# Generic dev runner for fkst packages.
#
#   scripts/run.sh test [package]
#       Run fkst-framework --self-test once, then conformance + test for every
#       package, or just one. This is the single CI and local test entrypoint.
#
#   scripts/run.sh run <package> <department> [event-json]
#       One-shot run a department against the REAL host environment via
#       `fkst-framework run`: decode emitted RAISED events and dump the <RT>
#       scratch tree. Generic across packages; pass package-specific config via
#       env, e.g.:
#         FKST_GITHUB_REPO=owner/repo scripts/run.sh run github-proxy github_poll
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
  sed -n '2,30p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'
}

cmd_test() {
  local target="${1:-}" ran=0 fail=0 pkg name
  local self_rt

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

  for pkg in "$ROOT"/packages/*/; do
    name="$(basename "$pkg")"
    if [ -n "$target" ] && [ "$name" != "$target" ]; then continue; fi
    echo "=== $name ==="
    ran=$((ran + 1))
    if ! "$BIN" conformance --project-root "$pkg" --package-root "$pkg"; then
      fail=$((fail + 1))
      continue
    fi
    if ! "$BIN" test --project-root "$pkg" --package-root "$pkg"; then
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
  if [ "$fail" -ne 0 ]; then
    echo "FAILED: $fail failure(s) across $ran package(s)" >&2; exit 1
  fi
  echo "OK: $ran package(s)"
}

cmd_run() {
  local pkg="${1:-}" dept="${2:-}" event="${3:-{\"payload\":{}}}"
  if [ -z "$pkg" ] || [ -z "$dept" ]; then
    echo "usage: scripts/run.sh run <package> <department> [event-json]" >&2; exit 1
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
  test) shift; resolve_bin; ensure_fresh_bin; cmd_test "$@" ;;
  run)  shift; resolve_bin; ensure_fresh_bin; cmd_run "$@" ;;
  supervise) shift; resolve_bin; ensure_fresh_bin; cmd_supervise "$@" ;;
  build) shift; cmd_build "$@" ;;
  -h|--help|help|"") usage ;;
  *) echo "unknown subcommand: $1" >&2; usage; exit 1 ;;
esac
