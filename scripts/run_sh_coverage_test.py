#!/usr/bin/env python3
"""Behavior tests for scripts/run.sh Lua coverage self-test wiring."""

from __future__ import annotations

import os
import stat
import subprocess
import tempfile
import textwrap
import unittest
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[1]


def write_executable(path: Path, text: str) -> None:
    path.write_text(text, encoding="utf-8")
    path.chmod(path.stat().st_mode | stat.S_IXUSR)


class RunShCoverageHarness:
    def __init__(self, bin_body: str) -> None:
        self.tmp = tempfile.TemporaryDirectory()
        self.root = Path(self.tmp.name)
        self.runtime = self.root / "runtime"
        self.mini_repo = self.root / "mini-repo"
        self.mini_repo_scripts = self.mini_repo / "scripts"
        self.mini_repo_scripts.mkdir(parents=True)
        self.argv_log = self.root / "argv.log"
        self.framework = self.root / "fkst-framework"
        write_executable(
            self.mini_repo_scripts / "check_repo.py",
            "#!/usr/bin/env python3\nraise SystemExit(0)\n",
        )
        write_executable(self.framework, bin_body)

    def close(self) -> None:
        self.tmp.cleanup()

    def run_function(self) -> subprocess.CompletedProcess[str]:
        env = os.environ.copy()
        env["BIN"] = str(self.framework)
        env["FKST_RUNTIME_ROOT"] = str(self.runtime)
        env["RUN_SH_COVERAGE_ARGV_LOG"] = str(self.argv_log)
        env["RUN_SH_COVERAGE_MINI_REPO"] = str(self.mini_repo)
        return subprocess.run(
            [
                "/bin/bash",
                "-c",
                textwrap.dedent(
                    """\
                    source scripts/run.sh
                    ROOT="$RUN_SH_COVERAGE_MINI_REPO"
                    run_self_test_with_optional_lua_coverage
                    """
                ),
            ],
            cwd=REPO_ROOT,
            env=env,
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            check=False,
        )

    def argv_lines(self) -> list[str]:
        if not self.argv_log.exists():
            return []
        return self.argv_log.read_text(encoding="utf-8").splitlines()


class RunShCoverageSelfTest(unittest.TestCase):
    def test_self_test_passes_coverage_flag_with_directory_value(self) -> None:
        h = RunShCoverageHarness(
            textwrap.dedent(
                """\
                #!/bin/sh
                printf '%s\\n' "$*" >> "$RUN_SH_COVERAGE_ARGV_LOG"
                if [ "$1" = "--self-test" ] && [ "$2" = "--coverage" ] && [ -n "${3:-}" ]; then
                  printf '{"files": []}\\n' > "$3/coverage.json"
                  exit 0
                fi
                echo "unexpected argv: $*" >&2
                exit 64
                """
            )
        )
        try:
            result = h.run_function()
            self.assertEqual(result.returncode, 0, result.stderr + result.stdout)
            self.assertEqual(len(h.argv_lines()), 1)
            argv = h.argv_lines()[0].split()
            self.assertGreaterEqual(len(argv), 3)
            self.assertEqual(argv[0:2], ["--self-test", "--coverage"])
            self.assertEqual(argv[2], str(h.runtime / "lua-coverage"))
        finally:
            h.close()

    def test_missing_coverage_value_propagates_without_plain_self_test_fallback(self) -> None:
        h = RunShCoverageHarness(
            textwrap.dedent(
                """\
                #!/bin/sh
                printf '%s\\n' "$*" >> "$RUN_SH_COVERAGE_ARGV_LOG"
                if [ "$1" = "--self-test" ] && [ "$2" = "--coverage" ]; then
                  echo "missing value for --coverage" >&2
                  exit 2
                fi
                if [ "$1" = "--self-test" ] && [ "$#" -eq 1 ]; then
                  echo "plain self-test fallback must not run" >&2
                  exit 0
                fi
                echo "unexpected argv: $*" >&2
                exit 64
                """
            )
        )
        try:
            result = h.run_function()
            self.assertNotEqual(result.returncode, 0, result.stderr + result.stdout)
            self.assertIn("missing value for --coverage", result.stderr)
            self.assertEqual(h.argv_lines(), [f"--self-test --coverage {h.runtime / 'lua-coverage'}"])
            self.assertNotIn("--self-test", h.argv_lines()[1:])
            self.assertNotIn("plain self-test fallback must not run", result.stderr + result.stdout)
        finally:
            h.close()


if __name__ == "__main__":
    unittest.main()
