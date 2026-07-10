#!/usr/bin/env bash
# Bounded parallel execution helpers for the run.sh test/check orchestration.
#
# The full gate `scripts/run.sh test` is a COMPLETE, order-free AND-fold over
# mutually-independent verification units: ~40 repo-check python processes in
# cmd_check and one engine process per package in cmd_test. AND is commutative, so
# running the units concurrently changes only wall-clock, never which units run or
# their pass/fail set. These helpers own that concurrency (bounded to logical cores),
# a deterministic replay of each unit's output, and per-unit hermetic runtime/durable
# roots so parallel package runs never share engine state. Sourced into run.sh's shell
# so the units inherit its functions and (dynamic-scoped) locals.

# Parallel pool size = logical core count (a hardware fact read from the OS, not a
# magic constant). Falls back conservatively when the count cannot be read.
detect_pool_size() {
  local n=""
  n="$(sysctl -n hw.logicalcpu 2>/dev/null || true)"
  [ -n "$n" ] || n="$(getconf _NPROCESSORS_ONLN 2>/dev/null || true)"
  [ -n "$n" ] || n="$(nproc 2>/dev/null || true)"
  case "$n" in ''|*[!0-9]*) n=4 ;; esac
  [ "$n" -ge 1 ] 2>/dev/null || n=4
  printf '%s\n' "$n"
}

# Run independent unit commands concurrently under a bounded worker pool, then replay
# each unit's captured output in submission order and AND-fold their exit codes. The
# verdict (count of failed units) is order-independent — parallelism changes only the
# wall-clock of independent work, never the pass/fail set. Units run as background
# subshells of the caller's shell, so they inherit every function and global defined
# there (no export or shell-out-to-self needed). Work-stealing throttle via `jobs -pr`
# keeps it bash-3.2 compatible (no `wait -n`).
# Args: <pool-size> <unit-command-string>...   Returns: count of failed units.
run_units_parallel() {
  local pool="$1"; shift
  local -a cmds=("$@")
  local n=${#cmds[@]}
  [ "$n" -gt 0 ] || return 0
  # Fail CLOSED on setup failure: run under `set +e` / left-of-|| where errexit is
  # suppressed, so an unchecked mktemp would leave $dir empty and route unit output to
  # `/$i.out` — silently truncating files at / on a writable-/ host (CI-as-root) and
  # potentially returning a false-green. An explicit check makes infra failure a failure.
  local dir
  if ! dir="$(mktemp -d "${TMPDIR:-/tmp}/fkst-units.XXXXXX")"; then
    echo "error: run_units_parallel could not create its work directory" >&2
    return 1
  fi
  local i j fails=0 rc running
  for (( i=0; i<n; i++ )); do
    # Throttle: launch the next unit only once a worker slot frees (true work-stealing).
    while :; do
      running="$(jobs -pr | wc -l | tr -d ' ')"
      [ "${running:-0}" -lt "$pool" ] && break
      sleep 0.05
    done
    ( set +e; eval "${cmds[$i]}" >"$dir/$i.out" 2>&1; printf '%s' "$?" >"$dir/$i.rc" ) &
  done
  wait
  for (( j=0; j<n; j++ )); do
    cat "$dir/$j.out" 2>/dev/null || true
    rc="$(cat "$dir/$j.rc" 2>/dev/null || printf '1')"
    [ "$rc" = 0 ] || fails=$(( fails + 1 ))
  done
  rm -rf "$dir"
  return "$fails"
}

# Run one package's conformance + test(s) with its OWN ephemeral runtime/durable roots,
# so packages running in parallel never share engine runtime/durable state (the tests'
# real filesystem IO is FKST_RUNTIME_ROOT-relative). The collection dirs (report_dir,
# coverage_report_dir), the roots_parent, and the failure filter are EXPLICIT arguments
# (the contract is in the signature, not a comment); the per-package roots are created
# UNDER roots_parent, which cmd_test registers in its EXIT trap so they are swept even if
# a unit subshell is killed. `--report-json`/`--coverage` land in the shared $name-keyed
# collection dirs for the post-join ratchet/G5 fan-in. Mirrors the former serial loop
# body exactly; returns 0 on success, 1 on any failure. `verbose` and the run_quiet_*/
# load_composed_test_roots functions and BIN are legitimately module-ambient.
#
# CONTRACT: unit commands (this and the cmd_check checks) must always `return`, never
# `exit` — an `exit` terminates the pool's capturing subshell before it records the exit
# code, which run_units_parallel then fail-closes as a failure. run_one_package must be
# invoked only via run_units_parallel (its FKST_RUNTIME_ROOT export relies on the subshell).
run_one_package() {
  local name="$1" pkg="$2" is_pkg_composed="$3"
  local report_dir="$4" coverage_report_dir="$5" roots_parent="$6" test_failure_filter="$7"
  local rt dur report_file coverage_dir test_project_root result=0
  local -a test_pkg_args
  # Fail CLOSED on root-setup failure: an unchecked mktemp under `set +e` would export
  # an EMPTY FKST_RUNTIME_ROOT/FKST_DURABLE_ROOT, silently defeating per-package isolation
  # (the engine may then fall back to a shared/default root). A failed setup must fail the unit.
  if ! rt="$(mktemp -d "$roots_parent/rt.XXXXXX")"; then
    echo "error: could not create runtime root for package $name" >&2
    return 1
  fi
  if ! dur="$(mktemp -d "$roots_parent/durable.XXXXXX")"; then
    echo "error: could not create durable root for package $name" >&2
    rm -rf "$rt"
    return 1
  fi
  export FKST_RUNTIME_ROOT="$rt" FKST_DURABLE_ROOT="$dur"
  echo "=== $name ==="
  if [ "$is_pkg_composed" -eq 1 ]; then
    echo "skip single-package conformance for composed package: $name"
  elif ! run_quiet_pass "$BIN" conformance --project-root "$pkg" --package-root "$pkg"; then
    result=1
  fi
  if [ "$result" -eq 0 ]; then
    report_file="$report_dir/$name.json"
    coverage_dir="$coverage_report_dir/$name"
    # Fail CLOSED if the coverage dir cannot be reset: otherwise a stale coverage.json from
    # a prior run could survive and be accepted below as this run's artifact.
    if ! rm -rf "$coverage_dir" || ! mkdir -p "$coverage_dir"; then
      echo "error: could not prepare coverage directory for package $name" >&2
      rm -rf "$rt" "$dur"
      return 1
    fi
    test_project_root="$pkg"; test_pkg_args=(--package-root "$pkg")
    if [ "$is_pkg_composed" -eq 1 ] && ! load_composed_test_roots normal "$name"; then
      result=1
    elif ! run_quiet_keep "$test_failure_filter" \
        "$BIN" test --project-root "$test_project_root" "${test_pkg_args[@]}" --report-json "$report_file" --coverage "$coverage_dir"; then
      result=1
    else
      if [ "$is_pkg_composed" -eq 1 ] && compgen -G "$pkg/tests/run_graph*_test.lua" >/dev/null; then
        if ! load_composed_test_roots graph "$name" || ! run_quiet_keep "$test_failure_filter" \
            "$BIN" test --project-root "$test_project_root" "${test_pkg_args[@]}" --report-json "$report_dir/$name.graph.json" --coverage "$coverage_dir.graph"; then
          result=1
        fi
      fi
      if [ "$result" -eq 0 ] && [ ! -f "$coverage_dir/coverage.json" ]; then
        echo "error: fkst-framework test --coverage did not write coverage.json for $name in $coverage_dir" >&2
        result=1
      fi
    fi
  fi
  rm -rf "$rt" "$dur"
  return "$result"
}
