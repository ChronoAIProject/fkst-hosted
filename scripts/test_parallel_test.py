#!/usr/bin/env python3
"""Behavior tests for scripts/test_parallel.sh — the bounded parallel executor that
backs scripts/run.sh test/check.

These exercise the gate's OWN failure-propagation and ordering contract, which a green
full run never samples (a full run only walks the all-pass path). Without this, a future
edit that neutered `return "$fails"` or the rc-missing fail-closed fallback would keep CI
green while silently deadening the parallel gate."""
import subprocess
import unittest
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[1]
TEST_PARALLEL = REPO_ROOT / "scripts" / "test_parallel.sh"


def _run(snippet: str) -> subprocess.CompletedProcess:
    """Source test_parallel.sh and run a bash snippet; return the completed process."""
    script = f'set -uo pipefail\n. "{TEST_PARALLEL}"\n{snippet}\n'
    return subprocess.run(
        ["/bin/bash", "-c", script],
        capture_output=True,
        text=True,
        cwd=REPO_ROOT,
    )


class DetectPoolSizeTest(unittest.TestCase):
    def test_returns_a_positive_integer(self) -> None:
        result = _run("detect_pool_size")
        self.assertEqual(result.returncode, 0, result.stderr)
        value = result.stdout.strip()
        self.assertTrue(value.isdigit(), f"not an integer: {value!r}")
        self.assertGreaterEqual(int(value), 1)


class RunUnitsParallelTest(unittest.TestCase):
    def test_empty_unit_list_returns_zero(self) -> None:
        result = _run('run_units_parallel 4; echo "rc=$?"')
        self.assertIn("rc=0", result.stdout)

    def test_all_pass_returns_zero_failures(self) -> None:
        result = _run("run_units_parallel 3 'echo a' 'echo b' 'echo c'; echo \"rc=$?\"")
        self.assertIn("rc=0", result.stdout)

    def test_fail_count_via_recorded_nonzero_rc(self) -> None:
        # THE PRIMARY PRODUCTION PATH: a unit that returns nonzero WITHOUT exiting its
        # capturing subshell (an external process like `python3 -B check.py`, or
        # run_one_package which ends in `return`). The exit code is recorded by the
        # `printf '%s' "$?"` line, so this pins that line — a fail-open mutation of it
        # (record 0 instead of $?) makes every real failing check count as pass. 2 of 5
        # units fail via a recorded rc -> returned fail count must be exactly 2.
        result = _run(
            "run_units_parallel 3 'true' 'false' 'true' "
            "'python3 -c \"import sys; sys.exit(3)\"' 'true'; echo \"rc=$?\""
        )
        self.assertIn("rc=2", result.stdout)

    def test_fail_count_via_subshell_exit_missing_rc(self) -> None:
        # THE OTHER PATH: a unit whose eval'd command `exit`s terminates the capturing
        # subshell before the rc line, so no rc file is written -> fail-closed as a
        # failure. 2 of 5 units self-exit -> returned fail count must be exactly 2.
        result = _run(
            "run_units_parallel 3 'true' 'exit 1' 'true' 'exit 2' 'true'; echo \"rc=$?\""
        )
        self.assertIn("rc=2", result.stdout)

    def test_output_is_replayed_in_submission_order_regardless_of_duration(self) -> None:
        # U0 sleeps longest but is submitted first: output MUST still be U0..U4 in order,
        # proving deterministic submission-order replay (not completion order).
        result = _run(
            "run_units_parallel 5 "
            "'sleep 0.3; echo U0' 'echo U1' 'echo U2' 'sleep 0.1; echo U3' 'echo U4'"
        )
        lines = [ln for ln in result.stdout.splitlines() if ln.startswith("U")]
        self.assertEqual(lines, ["U0", "U1", "U2", "U3", "U4"])

    def test_work_dir_setup_failure_fails_closed(self) -> None:
        # If run_units_parallel cannot create its work directory (mktemp fails), it must
        # FAIL CLOSED — return nonzero with a diagnostic — never silently route unit
        # output to `/$i.out` (which on a writable-/ host, e.g. CI-as-root, would truncate
        # files at / and can return a false-green). Force mktemp to fail via a bad TMPDIR.
        result = _run(
            'TMPDIR=/no/such/dir/xyz run_units_parallel 2 '
            "'echo passA' 'echo passB'; echo \"rc=$?\""
        )
        self.assertNotIn("rc=0", result.stdout)
        self.assertIn("could not create its work directory", result.stderr + result.stdout)

    def test_unit_whose_subshell_exits_before_recording_rc_fails_closed(self) -> None:
        # A unit whose eval'd command `exit`s terminates the capturing subshell BEFORE
        # the `printf "$?" > rc` line runs, so no rc file is written. That abnormal
        # termination must fail-closed: the missing rc is counted as a failure, never a
        # silent pass. Here unit 1 self-exits; units 0 and 2 pass -> exactly 1 failure.
        result = _run(
            "run_units_parallel 2 'true' 'exit 0' 'true'; echo \"rc=$?\""
        )
        self.assertIn("rc=1", result.stdout)


if __name__ == "__main__":
    unittest.main()
