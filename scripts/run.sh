#!/usr/bin/env bash
# Generic dev runner for fkst packages.
#
#   scripts/run.sh test [-v|--verbose] [package]
#       Run self-test, flat package conformance, package tests, and composed
#       graph conformance. Tests use fresh runtime/durable roots and keep only
#       failure-relevant lines unless -v/--verbose or FKST_TEST_VERBOSE=1 is set.
#
#   scripts/run.sh check
#       Run hermetic repository checks and engine workspace dependency validation.
#
#   scripts/run.sh host --host-root <HOST> [--platform-root <PKGSRC>] [--local-packages <dir>] -- <check|test|supervise [args]>
#       Run shared fkst-packages orchestration for a host repo. The host passes
#       only its root/config; this runner owns BIN resolution, source ratchets,
#       engine package-root wiring, and host_run.sh supervise delegation.
#
#   scripts/run.sh doctor
#       Run read-only preflight checks for git/cargo/rustc, fkst-framework BIN,
#       codex, gh auth, and relevant FKST_* host facts.
#
#   scripts/run.sh doctor github-devloop-ops
#       Run the read-only package-side saga doctor against the configured
#       running GitHub repository. Exact engine queue/DLQ depths remain
#       unavailable here and need fkst-framework doctor support.
#
#   scripts/run.sh board [--refresh] [--ttl seconds] [--stall seconds]
#       Render the local github-devloop observability board from engine observe
#       data, using a local non-authoritative TTL cache unless --refresh is set.
#
#   scripts/run.sh ratchet-migration-dry-run <891|892> [--slice-size N]
#       Print a deterministic child issue body for a code-owned allowlist ratchet
#       parent. This is read-only and never creates issues, writes comments, closes
#       parents, or runs issue-provided inventory commands.
#
#   scripts/run.sh health [--refresh] [--ttl seconds] [--stall seconds]
#       Print only the current HEALTHY / anomaly verdict from the board renderer.
#
#   scripts/run.sh test-composed
#       Run only composed graph conformance for composed package graphs.
#
#   scripts/run.sh test-affected
#       Run scoped local verification for paths changed from the integration base.
#
#   scripts/run.sh run <package> <department> [event-json]
#   scripts/run.sh run <package> <department> --event-file <path>
#       One-shot run a department through fkst-framework run, decode emitted
#       RAISED events, and dump the runtime scratch tree. Never sets
#       FKST_GITHUB_WRITE.
#
#   scripts/run.sh supervise --project-root <HOST> --platform-root <PKGSRC> --platform-packages "<names>" [--host-packages "<names>"] --durable-root <path> [--runtime-root <fresh-scratch-root>] [--restart]
#       Start the real fkst-framework supervise event loop for one host. Runtime
#       root is scratch and defaults to a fresh temp dir; explicit --runtime-root
#       is used as the fresh scratch root for this launch.
#       Platform package roots are resolved from the target fkst.workspace.toml
#       and fkst.lock, not from ad hoc package-root construction.
#       Durable root is mandatory and reused. --restart SIGKILLs the prior host-run supervise
#       recorded for that durable root. FKST_GITHUB_WRITE passes through
#       (unset = dry-run).
#
#   scripts/run.sh supervise <package>
#       Backward-compatible package-local supervise wrapper. Uses .fkst/run/runtime
#       and .fkst/run/durable by default and requires FKST_RATE_POOL_ROOT from the
#       host so named external-command rate pools are shared across instances.
#
#   scripts/run.sh build
#       Local-only helper: update the fkst-substrate dev checkout and build
#       fkst-framework. test/run/supervise ensure a traceable local BIN is built
#       from the current fkst-substrate working tree before running.
#
# fkst-framework binary resolution (priority): $BIN > repo .fkst/env `BIN=` > PATH >
# sibling ../fkst-substrate/target/debug/fkst-framework > pinned source cache
# clone/build fallback.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
FKST_DIR="$ROOT/.fkst"
SOURCE_PACKAGES_ROOT="$ROOT/packages"
LOCAL_PACKAGES_ROOT="$FKST_DIR/local-packages"
EXTERNAL_PACKAGES_ROOT="$FKST_DIR/packages"
DEFAULT_RUNTIME_ROOT="$FKST_DIR/run/runtime"
DEFAULT_DURABLE_ROOT="$FKST_DIR/run/durable"

# shellcheck source=scripts/bin_bootstrap.sh
. "$ROOT/scripts/bin_bootstrap.sh"
# shellcheck source=scripts/host_run.sh
. "$ROOT/scripts/host_run.sh"
# shellcheck source=scripts/host_entry.sh
. "$ROOT/scripts/host_entry.sh"
# shellcheck source=scripts/composed_manifest.sh
. "$ROOT/scripts/composed_manifest.sh"
# shellcheck source=scripts/test_affected.sh
. "$ROOT/scripts/test_affected.sh"

resolve_bin() {
  if ! resolve_bin_contract "$ROOT" "bootstrap"; then
    echo "error: $RESOLVE_BIN_ERROR" >&2
    if [ -n "${CI:-}" ] || [ -n "${GITHUB_ACTIONS:-}" ]; then
      echo "  CI must build fkst-substrate and inject BIN; scripts/run.sh will not build in CI." >&2
    fi
    exit 1
  fi
  BIN="$RESOLVED_BIN"
  export BIN
}

shell_single_quote() {
  printf "'%s'" "$(printf '%s' "$1" | sed "s/'/'\\\\''/g")"
}

default_board_cmd() {
  printf 'FKST_NO_AUTOBUILD=1 %s board' "$(shell_single_quote "$ROOT/scripts/run.sh")"
}

competence_gate_base_ref() {
  if [ -n "${FKST_COMPETENCE_BASE_REF:-}" ]; then
    printf '%s\n' "$FKST_COMPETENCE_BASE_REF"
    return 0
  fi
  if [ -n "${GITHUB_BASE_REF:-}" ]; then
    if git -C "$ROOT" rev-parse --verify --quiet "origin/$GITHUB_BASE_REF" >/dev/null; then
      printf 'origin/%s\n' "$GITHUB_BASE_REF"
    else
      printf '%s\n' "$GITHUB_BASE_REF"
    fi
    return 0
  fi
  if git -C "$ROOT" rev-parse --verify --quiet origin/integration >/dev/null; then
    printf '%s\n' "origin/integration"
    return 0
  fi
  if git -C "$ROOT" rev-parse --verify --quiet integration >/dev/null; then
    printf '%s\n' "integration"
    return 0
  fi
  return 1
}

ensure_package_view() {
  mkdir -p "$FKST_DIR"
  ln -sfn ../packages "$LOCAL_PACKAGES_ROOT"
}

package_root_for_name() {
  local name="$1"
  if [ -d "$LOCAL_PACKAGES_ROOT/$name" ]; then
    printf '%s\n' "$LOCAL_PACKAGES_ROOT/$name"
    return 0
  fi
  if [ -d "$EXTERNAL_PACKAGES_ROOT/$name" ]; then
    printf '%s\n' "$EXTERNAL_PACKAGES_ROOT/$name"
    return 0
  fi
  return 1
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

# Warn — without git-pulling (doctrine: scripts/run.sh never pulls; only dogfood.sh
# sync does) — when the fkst-substrate checkout the BIN traces to is behind its
# origin/dev, so a silently-stale BIN cannot masquerade as fresh. The freshness
# build below builds from the checkout's CURRENT source; if that checkout is behind
# origin/dev, the resulting BIN is missing newer engine primitives (e.g. exec_argv)
# and the migrated gh/git argv paths fail under it. Local refs only (no network), so
# this is a best-effort hint that surfaces a behind checkout after any prior fetch.
warn_if_substrate_behind() {
  local substrate="$1" behind
  behind="$(git -C "$substrate" rev-list --count HEAD..origin/dev 2>/dev/null)" || behind=""
  if [ -n "$behind" ] && [ "$behind" -gt 0 ] 2>/dev/null; then
    echo "warning: fkst-substrate checkout '$substrate' is $behind commit(s) behind its origin/dev;" >&2
    echo "         the BIN may be stale (missing newer engine primitives). scripts/run.sh builds from the" >&2
    echo "         CURRENT checkout and does NOT git-pull — run 'dogfood.sh sync' (or git pull + rebuild) to refresh." >&2
  fi
}

ensure_fresh_bin() {
  if [ -n "${CI:-}" ] || [ -n "${GITHUB_ACTIONS:-}" ]; then
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
    if [ -z "${FKST_NO_AUTOBUILD:-}" ]; then
      echo "warning: cannot trace BIN to an fkst-substrate checkout; skipping freshness build: $BIN" >&2
    fi
    return 0
  fi

  # Surface a behind-substrate checkout REGARDLESS of FKST_NO_AUTOBUILD — that is
  # exactly the silently-stale-BIN case: if the build runs it builds from this same
  # behind checkout, and if it is skipped the existing BIN is at best this old.
  warn_if_substrate_behind "$substrate"

  if [ -n "${FKST_NO_AUTOBUILD:-}" ]; then
    echo "warning: FKST_NO_AUTOBUILD set; skipping fkst-framework freshness build" >&2
    return 0
  fi

  echo "ensuring fkst-framework is built from current source: $substrate" >&2
  local build_out
  if ! build_out="$(cargo build --manifest-path "$substrate/Cargo.toml" -p fkst-framework 2>&1)"; then
    printf '%s\n' "$build_out" >&2
    echo "error: fkst-framework freshness build failed; refusing to continue with a potentially stale BIN" >&2
    exit 1
  fi
}

usage() {
  sed -n '2,36p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'
}

cmd_check() {
  local fail=0 competence_base_ref=""
  python3 -B "$ROOT/scripts/check_repo.py" || fail=1
  python3 -B "$ROOT/scripts/ratchet_base_test.py" || fail=1
  python3 -B "$ROOT/scripts/check_repo_fkst_layout.py" || fail=1
  python3 -B "$ROOT/scripts/check_repo_dedup_test.py" || fail=1
  python3 -B "$ROOT/scripts/check_repo_content_truncation_test.py" || fail=1
  python3 -B "$ROOT/scripts/check_repo_coverage_test.py" || fail=1
  python3 -B "$ROOT/scripts/check_repo_devloop_godlib_test.py" || fail=1
  python3 -B "$ROOT/scripts/check_repo_devloop_installer_test.py" || fail=1
  python3 -B "$ROOT/scripts/check_repo_integration_coverage_test.py" || fail=1
  python3 -B "$ROOT/scripts/check_repo_intake_default_surface_test.py" || fail=1
  python3 -B "$ROOT/scripts/check_repo_producer_liveness_test.py" || fail=1
  python3 -B "$ROOT/scripts/check_repo_monotone_gate_test.py" || fail=1
  python3 -B "$ROOT/scripts/check_repo_test_graphql.py" || fail=1
  python3 -B "$ROOT/scripts/check_repo_interface_test.py" || fail=1
  python3 -B "$ROOT/scripts/lua_coverage_to_lcov_test.py" || fail=1
  python3 -B "$ROOT/scripts/check_repo_test.py" || fail=1; python3 -B "$ROOT/scripts/check_repo_shell_out_to_self_test.py" || fail=1; python3 -B "$ROOT/scripts/check_repo_hidden_state_test.py" || fail=1
  python3 -B "$ROOT/scripts/check_repo_std_dependency_model_test.py" || fail=1
  python3 -B "$ROOT/scripts/check_repo_saga_head_test.py" || fail=1
  python3 -B "$ROOT/scripts/check_repo_namespaced_queue_test.py" || fail=1
  python3 -B "$ROOT/scripts/check_repo_fkst_layout_test.py" || fail=1
  python3 -B "$ROOT/scripts/bin_cache_test.py" || fail=1
  python3 -B "$ROOT/scripts/bin_bootstrap_test.py" || fail=1
  python3 -B "$ROOT/scripts/host_entry_test.py" || fail=1
  python3 -B "$ROOT/scripts/host_run_test.py" || fail=1
  python3 -B "$ROOT/scripts/host_profile_scaffold_test.py" || fail=1
  python3 -B "$ROOT/scripts/host_run_equivalence_test.py" || fail=1
  python3 -B "$ROOT/scripts/run_sh_coverage_test.py" || fail=1
  python3 -B "$ROOT/scripts/run_sh_test_affected_test.py" || fail=1
  python3 -B "$ROOT/scripts/composed_manifest_test.py" || fail=1
  python3 -B "$ROOT/scripts/board_test.py" || fail=1
  python3 -B "$ROOT/scripts/dogfood_board_test.py" || fail=1
  python3 -B "$ROOT/scripts/doctor_test.py" || fail=1
  python3 -B "$ROOT/scripts/ratchet_migration_slicer_test.py" || fail=1
  if ! competence_base_ref="$(competence_gate_base_ref)"; then
    echo "error: competence gate requires FKST_COMPETENCE_BASE_REF, GITHUB_BASE_REF, or an integration ref" >&2
    fail=1
  else
    python3 -B "$ROOT/scripts/competence_gate.py" --base-ref "$competence_base_ref" || fail=1
  fi
  python3 -B "$ROOT/scripts/competence_gate_test.py" || fail=1
  if [ "$fail" -eq 0 ]; then
    resolve_bin
    "$BIN" deps --project-root "$ROOT" || fail=1
  fi
  return "$fail"
}

check_test_file_coverage() {
  local report_dir="$1" expected actual missing
  expected="$(mktemp "${TMPDIR:-/tmp}/fkst-test-files-expected.XXXXXX")"
  actual="$(mktemp "${TMPDIR:-/tmp}/fkst-test-files-actual.XXXXXX")"
  missing="$(mktemp "${TMPDIR:-/tmp}/fkst-test-files-missing.XXXXXX")"

  (
    cd "$ROOT"
    find "$SOURCE_PACKAGES_ROOT" \( -path '*/tests/*_test.lua' -o -path '*/departments/*/*_test.lua' \) -type f -print \
      | while IFS= read -r path; do printf 'packages/%s\n' "${path#"$SOURCE_PACKAGES_ROOT"/}"; done \
      | LC_ALL=C sort -u
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
  cat > "$probe_dir/fkst.workspace.toml" <<'TOML'
[workspace]
units = ["."]
TOML
  cat > "$probe_dir/fkst.toml" <<'TOML'
kind = "package"
name = "sdk-probe"

[code]
root = "."
TOML
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
  echo "OK: SDK primitive truncate_utf8 is available in BIN: $BIN"
}

run_self_test_with_optional_lua_coverage() {
  local coverage_dir="$FKST_RUNTIME_ROOT/lua-coverage" coverage_json out rc
  rm -rf "$coverage_dir"
  mkdir -p "$coverage_dir"
  set +e
  out="$(cd "$ROOT" && "$BIN" --self-test --coverage "$coverage_dir" 2>&1)"
  rc=$?
  set -e
  if [ "$rc" -eq 0 ]; then
    printf '%s\n' "$out"
    coverage_json="$coverage_dir/coverage.json"
    if [ ! -f "$coverage_json" ]; then
      echo "error: fkst-framework --self-test --coverage did not write coverage.json in $coverage_dir" >&2
      return 1
    fi
    return $?
  fi
  if printf '%s\n' "$out" | grep -Eq "(unknown|unrecognized).*--coverage"; then
    echo "warning: fkst-framework does not expose --self-test --coverage; skipping Lua coverage ratchet artifact collection" >&2
    "$BIN" --self-test
    return $?
  fi
  printf '%s\n' "$out" >&2
  return "$rc"
}

enforce_lua_coverage_ratchet() {
  local output="${FKST_LUA_COVERAGE_OUTPUT:-$FKST_RUNTIME_ROOT/lua-coverage/coverage.json}" inputs=() artifact package_name
  shift || true
  if [ "$#" -eq 0 ]; then
    echo "error: Lua coverage ratchet has no package coverage artifacts" >&2
    return 1
  fi
  for artifact in "$@"; do
    package_name="$(basename "$(dirname "$artifact")")"
    inputs+=("$package_name=$artifact")
  done
  FKST_LUA_COVERAGE_MERGED_OUTPUT="$output" python3 -B - "$ROOT" "${inputs[@]}" <<'PY'
import importlib.util
import os
import sys
from pathlib import Path

root = Path(sys.argv[1])
spec = importlib.util.spec_from_file_location("check_repo_coverage", root / "scripts" / "check_repo_coverage.py")
if spec is None or spec.loader is None:
    raise SystemExit("error: could not load scripts/check_repo_coverage.py")
coverage = importlib.util.module_from_spec(spec)
sys.modules[spec.name] = coverage
spec.loader.exec_module(coverage)
artifacts = [coverage.parse_covered_json_arg(value) for value in sys.argv[2:]]
count = coverage.write_canonical_coverage_json(
    coverage.merge_covered_sets(artifacts),
    Path(os.environ["FKST_LUA_COVERAGE_MERGED_OUTPUT"]),
    root,
)
print(f"wrote {count} file(s) to {os.environ['FKST_LUA_COVERAGE_MERGED_OUTPUT']}")
PY
  if [ ! -f "$output" ]; then
    echo "error: Lua coverage ratchet did not write coverage artifact: $output" >&2
    return 1
  fi
  FKST_LUA_COVERAGE_JSON="$output" python3 -B "$ROOT/scripts/check_repo.py"
}

# Run "$@"; unless verbose (cmd_test's flag), drop advisory `PASS` lines from its
# combined output so only failures surface. Returns the command's own exit code
# (via PIPESTATUS, not grep's). The `set +e`/`set -e` guard makes it safe in any
# caller context: the inner grep matching nothing on an all-pass run must not
# trip the script-wide `set -e`.
run_quiet_pass() {
  if [ -n "${verbose:-}${FKST_TEST_VERBOSE:-}" ]; then "$@"; return $?; fi
  local rc
  set +e
  "$@" 2>&1 | grep -vE '^PASS '
  rc=${PIPESTATUS[0]}
  set -e
  return "$rc"
}

# Run "$2..."; unless verbose, KEEP only stdout lines matching the regex in $1
# (the inverse of run_quiet_pass — allowlist for the noisy engine test stream).
# Returns the command's own exit code via PIPESTATUS, not grep's, so an all-pass
# run (grep still matches the tally) and a failing run both report correctly.
# Same `set +e`/`set -e` guard so a failing package neither aborts the run nor is
# swallowed: the loop continues and the count stays accurate.
run_quiet_keep() {
  local keep="$1"; shift
  if [ -n "${verbose:-}${FKST_TEST_VERBOSE:-}" ]; then "$@"; return $?; fi
  local rc
  set +e
  "$@" 2>&1 | grep -E -- "$keep"
  rc=${PIPESTATUS[0]}
  set -e
  return "$rc"
}

load_composed_test_roots() { local script; script="$(bash "$ROOT/scripts/composed_test_graph_roots.sh" "$1" "$2")" || return 1; eval "$script"; }

cmd_test() {
  local target="" ran=0 fail=0 pkg name verbose="${FKST_TEST_VERBOSE:-}" rc is_pkg_composed
  local report_dir report_file coverage_report_dir coverage_dir coverage_file
  local coverage_artifacts=()
  local test_project_root test_pkg_args
  # Keep failure-relevant lines only unless verbose; per-test FAIL is anchored so
  # expected error-path logs containing tag=FAILURE do not match.
  local test_failure_filter='^FAIL |passed, [0-9]+ failed|panic'
  while [ $# -gt 0 ]; do
    case "$1" in
      -v|--verbose) verbose=1 ;;
      -*) echo "unknown test flag: $1" >&2; exit 2 ;;
      *) target="$1" ;;
    esac
    shift
  done

  trap 'rm -rf "${TEST_HERMETIC_RUNTIME_ROOT:-}" "${TEST_HERMETIC_DURABLE_ROOT:-}"' EXIT
  TEST_HERMETIC_RUNTIME_ROOT="$(mktemp -d "${TMPDIR:-/tmp}/fkst-test-rt.XXXXXX")"
  TEST_HERMETIC_DURABLE_ROOT="$(mktemp -d "${TMPDIR:-/tmp}/fkst-test-durable.XXXXXX")"
  export FKST_RUNTIME_ROOT="$TEST_HERMETIC_RUNTIME_ROOT"
  export FKST_DURABLE_ROOT="$TEST_HERMETIC_DURABLE_ROOT"
  unset FKST_GITHUB_WRITE
  unset FKST_SUPERVISOR_PID
  echo "test hermetic: FKST_RUNTIME_ROOT=$FKST_RUNTIME_ROOT FKST_DURABLE_ROOT=$FKST_DURABLE_ROOT (ambient overridden)"

  report_dir="$(mktemp -d "${TMPDIR:-/tmp}/fkst-test-reports.XXXXXX")"
  coverage_report_dir="$FKST_RUNTIME_ROOT/package-lua-coverage"

  echo "=== self-test ==="
  if ! run_self_test_with_optional_lua_coverage; then
    fail=$((fail + 1))
  fi

  echo "=== sdk-primitives ==="
  if ! run_quiet_pass check_sdk_primitives; then
    fail=$((fail + 1))
  fi

  ensure_package_view
  for src_pkg in "$SOURCE_PACKAGES_ROOT"/*/; do
    [ -d "$src_pkg" ] || continue
    name="$(basename "$src_pkg")"
    pkg="$LOCAL_PACKAGES_ROOT/$name"
    [ -d "$pkg" ] || continue
    if [ -n "$target" ] && [ "$name" != "$target" ]; then continue; fi
    echo "=== $name ==="
    ran=$((ran + 1))
    rc=0; is_composed "$pkg" || rc=$?
    case "$rc" in
      0) is_pkg_composed=1 ;;
      1) is_pkg_composed=0 ;;
      *) echo "error: failed to read package composition for $pkg" >&2; fail=$((fail + 1)); continue ;;
    esac
    if [ "$is_pkg_composed" -eq 1 ]; then
      echo "skip single-package conformance for composed package: $name"
    else
      if ! run_quiet_pass "$BIN" conformance --project-root "$pkg" --package-root "$pkg"; then
        fail=$((fail + 1))
        continue
      fi
    fi
    report_file="$report_dir/$name.json"
    coverage_dir="$coverage_report_dir/$name"
    rm -rf "$coverage_dir"
    mkdir -p "$coverage_dir"
    test_project_root="$pkg"; test_pkg_args=(--package-root "$pkg")
    if [ "$is_pkg_composed" -eq 1 ]; then
      if ! load_composed_test_roots normal "$name"; then fail=$((fail + 1)); continue; fi
    fi
    # Default-quiet: keep only failure-relevant lines (the --report-json that
    # drives the tally and G5 coverage is unaffected). run_quiet_keep is called
    # from `if !` so the inner pipe never trips `set -e` on a failing package;
    # the loop continues, the count is correct, and FAILED: still prints.
    if ! run_quiet_keep "$test_failure_filter" \
        "$BIN" test --project-root "$test_project_root" "${test_pkg_args[@]}" --report-json "$report_file" --coverage "$coverage_dir"; then
      fail=$((fail + 1))
    else
      if [ "$is_pkg_composed" -eq 1 ] && compgen -G "$pkg/tests/run_graph*_test.lua" >/dev/null; then
        if ! load_composed_test_roots graph "$name" || ! run_quiet_keep "$test_failure_filter" \
            "$BIN" test --project-root "$test_project_root" "${test_pkg_args[@]}" --report-json "$report_dir/$name.graph.json" --coverage "$coverage_dir.graph"; then
          fail=$((fail + 1))
          continue
        fi
      fi
      coverage_file="$coverage_dir/coverage.json"
      if [ ! -f "$coverage_file" ]; then
        echo "error: fkst-framework test --coverage did not write coverage.json for $name in $coverage_dir" >&2
        fail=$((fail + 1))
      else
        coverage_artifacts+=("$coverage_file")
      fi
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
      if ! enforce_lua_coverage_ratchet -- "${coverage_artifacts[@]}"; then
        fail=$((fail + 1))
      fi
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
  local name="$1" pkg dep deps rc
  pkg="$(package_root_for_name "$name")" || { echo "error: composed package dependency not found: $name" >&2; return 1; }
  [ -d "$pkg" ] || { echo "error: composed package dependency not found: $name" >&2; return 1; }
  case " ${COMPOSED_SEEN[*]-} " in
    *" $name "*) return 0 ;;
  esac
  COMPOSED_SEEN+=("$name")
  set +e; deps="$(composition_siblings_of "$pkg")"; rc=$?; set -e
  case "$rc" in
    0)
      while IFS= read -r dep || [ -n "$dep" ]; do
        [ -n "$dep" ] || continue
        collect_composed_package "$dep" || return 1
      done <<< "$deps"
      ;;
    1) return 0 ;;
    *) echo "error: failed to read package composition for $pkg" >&2; return 1 ;;
  esac
}

cmd_test_composed() {
  local pkg name args project_root rc
  ensure_package_view
  COMPOSED_SEEN=()
  for pkg in "$LOCAL_PACKAGES_ROOT"/*/ "$EXTERNAL_PACKAGES_ROOT"/*/; do
    [ -d "$pkg" ] || continue
    rc=0; is_composed "$pkg" || rc=$?
    case "$rc" in
      0) ;;
      1) continue ;;
      *) echo "error: failed to read package composition for $pkg" >&2; return 1 ;;
    esac
    name="$(basename "$pkg")"
    collect_composed_package "$name" || return 1
  done
  if [ "${#COMPOSED_SEEN[@]}" -eq 0 ]; then
    echo "no composed packages matched"
    return 0
  fi

  args=()
  project_root="$(package_root_for_name "${COMPOSED_SEEN[0]}")" || return 1
  for name in "${COMPOSED_SEEN[@]}"; do
    pkg="$(package_root_for_name "$name")" || return 1
    args+=(--package-root "$pkg")
  done
  for pkg in "$LOCAL_PACKAGES_ROOT"/*/ "$EXTERNAL_PACKAGES_ROOT"/*/; do
    [ -d "$pkg" ] || continue
    case " ${COMPOSED_SEEN[*]} " in
      *" $(basename "$pkg") "*) continue ;;
    esac
    args+=(--package-root "${pkg%/}")
  done
  echo "=== composed conformance ==="
  run_quiet_pass "$BIN" conformance --project-root "$project_root" "${args[@]}"
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

  ensure_package_view
  local pkgdir lua args rootdir
  pkgdir="$(package_root_for_name "$pkg")" || { echo "error: no package named $pkg" >&2; exit 1; }
  lua="$pkgdir/departments/$dept/main.lua"
  [ -f "$lua" ] || { echo "error: no department at $lua" >&2; exit 1; }

  local rt fresh=0
  if [ -n "${FKST_RUNTIME_ROOT:-}" ]; then
    rt="$FKST_RUNTIME_ROOT"
  else
    rt="$DEFAULT_RUNTIME_ROOT"; fresh=1
    mkdir -p "$rt"
  fi
  export FKST_RUNTIME_ROOT="$rt"
  export FKST_DEVLOOP_BOARD_CMD="${FKST_DEVLOOP_BOARD_CMD:-$(default_board_cmd)}"

  echo "BIN=$BIN"
  echo "run $pkg/$dept  FKST_RUNTIME_ROOT=$rt${fresh:+ (fresh)}"
  if [ -n "${FKST_GITHUB_REPO:-}" ]; then echo "FKST_GITHUB_REPO=$FKST_GITHUB_REPO"; fi

  # Capture rc without set -e aborting at the assignment, so failure logs and
  # any partial RAISED/<RT> still print; propagate rc as the run's exit.
  local out rc=0
  args=("$BIN" run "$lua" --project-root "$ROOT")
  for rootdir in "$LOCAL_PACKAGES_ROOT"/*/ "$EXTERNAL_PACKAGES_ROOT"/*/; do
    [ -d "$rootdir" ] || continue
    args+=(--package-root "${rootdir%/}")
  done
  args+=(--owner-namespace "$pkg" --event "$event")
  out="$("${args[@]}" 2>&1)" || rc=$?

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

cmd_doctor() {
  if [ "$#" -eq 0 ]; then
    "$BASH" "$ROOT/scripts/doctor.sh"
    return $?
  fi

  local pkg="${1:-}"
  shift
  case "$pkg" in
    github-devloop-ops)
      if [ "$#" -ne 0 ]; then
        echo "usage: scripts/run.sh doctor github-devloop-ops" >&2
        exit 2
      fi
      resolve_bin
      ensure_fresh_bin
      ensure_package_view
      local pkgdir rootdir args
      pkgdir="$(package_root_for_name github-devloop-ops)" || { echo "error: no package named github-devloop-ops" >&2; exit 1; }
      local lua="$pkgdir/departments/doctor/main.lua"
      [ -f "$lua" ] || { echo "error: no saga doctor at $lua" >&2; exit 1; }
      args=("$BIN" run "$lua" --project-root "$ROOT")
      for rootdir in "$LOCAL_PACKAGES_ROOT"/*/ "$EXTERNAL_PACKAGES_ROOT"/*/; do
        [ -d "$rootdir" ] || continue
        args+=(--package-root "${rootdir%/}")
      done
      args+=(--owner-namespace github-devloop-ops --event '{"queue":"devloop_doctor_tick","payload":{}}')
      "${args[@]}" \
        | grep -vE '^RAISED:'
      ;;
    --running|--system)
      if [ "${1:-}" != "github-devloop-ops" ]; then
        echo "usage: scripts/run.sh doctor [github-devloop-ops|--running github-devloop-ops|--system github-devloop-ops]" >&2
        exit 2
      fi
      shift
      cmd_doctor github-devloop-ops "$@"
      ;;
    *)
      echo "usage: scripts/run.sh doctor [github-devloop-ops|--running github-devloop-ops|--system github-devloop-ops]" >&2
      exit 2
      ;;
  esac
}

cmd_board() {
  local durable="${FKST_DURABLE_ROOT:-$DEFAULT_DURABLE_ROOT}"
  local cache="$FKST_DIR/run/board-cache.json"
  python3 -B "$ROOT/scripts/board.py" \
    --bin "$BIN" \
    --durable-root "$durable" \
    --cache "$cache" \
    "$@"
}

cmd_health() {
  cmd_board --health "$@"
}

cmd_ratchet_migration_dry_run() {
  python3 -B "$ROOT/packages/github-ratchet-migration-slicer/tools/ratchet_migration_slicer.py" --repo-root "$ROOT" "$@"
}

cmd_supervise_old() {
  local pkg="${1:-}"
  if [ -z "$pkg" ]; then
    echo "usage: scripts/run.sh supervise <package>" >&2; exit 1
  fi
  if [ -z "${FKST_RATE_POOL_ROOT:-}" ]; then
    echo "error: FKST_RATE_POOL_ROOT is required for supervise so gh rate pools share one host-stable budget" >&2
    echo "  set FKST_RATE_POOL_ROOT to the same host-stable directory for every supervise instance that spends the GitHub quota" >&2
    exit 1
  fi
  case "$FKST_RATE_POOL_ROOT" in
    /*) ;;
    *)
      echo "error: FKST_RATE_POOL_ROOT must be an absolute host-stable directory path" >&2
      exit 1
      ;;
  esac
  ensure_package_view
  local pkgdir rootdir args
  pkgdir="$(package_root_for_name "$pkg")" || { echo "error: no package named $pkg" >&2; exit 1; }
  [ -d "$pkgdir" ] || { echo "error: no package at $pkgdir" >&2; exit 1; }

  local project_root rt durable
  project_root="${FKST_PROJECT_ROOT:-$pkgdir}"
  rt="${FKST_RUNTIME_ROOT:-$DEFAULT_RUNTIME_ROOT}"
  durable="${FKST_DURABLE_ROOT:-$DEFAULT_DURABLE_ROOT}"
  mkdir -p "$rt" "$durable"
  if [ "$rt" = "$durable" ]; then
    echo "error: FKST_RUNTIME_ROOT and FKST_DURABLE_ROOT resolved to the same directory" >&2
    exit 1
  fi
  export FKST_RUNTIME_ROOT="$rt"
  export FKST_DURABLE_ROOT="$durable"
  export FKST_DEVLOOP_BOARD_CMD="${FKST_DEVLOOP_BOARD_CMD:-$(default_board_cmd)}"

  echo "BIN=$BIN"
  echo "FKST_RUNTIME_ROOT=$FKST_RUNTIME_ROOT"
  echo "FKST_DURABLE_ROOT=$FKST_DURABLE_ROOT"
  echo "FKST_RATE_POOL_ROOT=$FKST_RATE_POOL_ROOT"
  echo "This starts the real supervise event loop in the foreground. Press Ctrl-C to stop."
  args=("$BIN" supervise --project-root "$project_root")
  for rootdir in "$LOCAL_PACKAGES_ROOT"/*/ "$EXTERNAL_PACKAGES_ROOT"/*/; do
    [ -d "$rootdir" ] || continue
    args+=(--package-root "${rootdir%/}")
  done
  args+=(--framework-bin "$BIN")
  echo "exec: ${args[*]}"
  exec "${args[@]}"
}

cmd_supervise() {
  case "${1:-}" in
    --*) host_run_supervise_contract "$@" ;;
    *) cmd_supervise_old "$@" ;;
  esac
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

main() {
  case "${1:-}" in
    check) shift; cmd_check "$@" ;;
    host) shift; cmd_host "$@" ;;
    doctor) shift; cmd_doctor "$@" ;;
    board) shift; resolve_bin; ensure_fresh_bin; cmd_board "$@" ;;
    health) shift; resolve_bin; ensure_fresh_bin; cmd_health "$@" ;;
    ratchet-migration-dry-run) shift; cmd_ratchet_migration_dry_run "$@" ;;
    test) shift
      # Quiet cmd_check's advisory warnings during a test run unless verbose;
      # surface its full output only when it hard-fails (non-zero). `run.sh check`
      # and `test -v`/FKST_TEST_VERBOSE=1 still show every warning.
      case " $* " in *" -v "*|*" --verbose "*) _tv=1 ;; *) _tv="${FKST_TEST_VERBOSE:-}" ;; esac
      if [ -n "$_tv" ]; then
        cmd_check
      elif ! _chk_out="$(cmd_check 2>&1)"; then
        printf '%s\n' "$_chk_out"; exit 1
      fi
      resolve_bin; ensure_fresh_bin; cmd_test "$@" ;;
    test-affected) shift; cmd_test_affected "$@" ;;
    test-composed) shift; cmd_check; resolve_bin; ensure_fresh_bin; cmd_test_composed "$@" ;;
    run)  shift; resolve_bin; ensure_fresh_bin; cmd_run "$@" ;;
    supervise) shift; resolve_bin; ensure_fresh_bin; cmd_supervise "$@" ;;
    build) shift; cmd_build "$@" ;;
    -h|--help|help|"") usage ;;
    *) echo "unknown subcommand: $1" >&2; usage; exit 1 ;;
  esac
}

if [[ "${BASH_SOURCE[0]}" == "$0" ]]; then
  main "$@"
fi
