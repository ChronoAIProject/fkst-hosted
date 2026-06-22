#!/usr/bin/env python3
"""Behavior tests for scripts/run.sh Lua coverage self-test wiring."""

from __future__ import annotations

import os
import json
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


class RunShCoverageRatchetHarness:
    def __init__(self) -> None:
        self.tmp = tempfile.TemporaryDirectory()
        self.root = Path(self.tmp.name)
        self.runtime = self.root / "runtime"
        self.repo = self.root / "repo"
        self.pkg = self.repo / "packages" / "example"
        self.scripts = self.repo / "scripts"
        self.scripts.mkdir(parents=True)
        self.pkg.mkdir(parents=True)
        self.pkg_coverage = self.runtime / "package-lua-coverage" / "example"
        self.pkg_coverage.mkdir(parents=True)
        self.check_env_log = self.root / "check-env.log"
        (self.repo / "libraries" / "forge").mkdir(parents=True)
        (self.pkg / "core.lua").write_text(
            "local M = {}\nfunction M.covered()\n  return 1\nend\nreturn M\n",
            encoding="utf-8",
        )
        (self.pkg_coverage / "coverage.json").write_text(
            json_text({"core.lua": {"covered_lines": [1, 2, 3, 5]}}),
            encoding="utf-8",
        )
        (self.scripts / "check_repo_coverage.py").write_text(
            (REPO_ROOT / "scripts" / "check_repo_coverage.py").read_text(encoding="utf-8"),
            encoding="utf-8",
        )
        write_executable(
            self.scripts / "check_repo.py",
            "#!/usr/bin/env python3\n"
            "import os\n"
            "from pathlib import Path\n"
            "Path(os.environ['RUN_SH_COVERAGE_CHECK_ENV_LOG']).write_text(os.environ.get('FKST_LUA_COVERAGE_JSON', ''), encoding='utf-8')\n"
            "raise SystemExit(0)\n",
        )

    def close(self) -> None:
        self.tmp.cleanup()

    def run_function(self, output: Path | None = None) -> subprocess.CompletedProcess[str]:
        env = os.environ.copy()
        env["FKST_RUNTIME_ROOT"] = str(self.runtime)
        env["RUN_SH_COVERAGE_MINI_REPO"] = str(self.repo)
        env["RUN_SH_COVERAGE_CHECK_ENV_LOG"] = str(self.check_env_log)
        if output is not None:
            env["FKST_LUA_COVERAGE_OUTPUT"] = str(output)
        return subprocess.run(
            [
                "/bin/bash",
                "-c",
                textwrap.dedent(
                    """\
                    source scripts/run.sh
                    ROOT="$RUN_SH_COVERAGE_MINI_REPO"
                    enforce_lua_coverage_ratchet -- "$FKST_RUNTIME_ROOT/package-lua-coverage/example/coverage.json"
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


def json_text(data: object) -> str:
    return json.dumps(data) + "\n"


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

    def test_unknown_coverage_flag_falls_back_before_ratchet_is_enabled(self) -> None:
        h = RunShCoverageHarness(
            textwrap.dedent(
                """\
                #!/bin/sh
                printf '%s\\n' "$*" >> "$RUN_SH_COVERAGE_ARGV_LOG"
                if [ "$1" = "--self-test" ] && [ "$2" = "--coverage" ]; then
                  echo "unrecognized option --coverage" >&2
                  exit 2
                fi
                if [ "$1" = "--self-test" ] && [ "$#" -eq 1 ]; then
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
            self.assertEqual(
                h.argv_lines(),
                [f"--self-test --coverage {h.runtime / 'lua-coverage'}", "--self-test"],
            )
            self.assertIn("skipping Lua coverage ratchet artifact collection", result.stderr)
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

    def test_unknown_self_test_coverage_flag_still_falls_back_when_ratchet_is_enabled(self) -> None:
        h = RunShCoverageHarness(
            textwrap.dedent(
                """\
                #!/bin/sh
                printf '%s\\n' "$*" >> "$RUN_SH_COVERAGE_ARGV_LOG"
                if [ "$1" = "--self-test" ] && [ "$2" = "--coverage" ]; then
                  echo "unrecognized option --coverage" >&2
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
            (h.mini_repo / "migration").mkdir()
            (h.mini_repo / "migration" / "coverage-uncovered.required").write_text("", encoding="utf-8")

            result = h.run_function()

            self.assertEqual(result.returncode, 0, result.stderr + result.stdout)
            self.assertEqual(
                h.argv_lines(),
                [f"--self-test --coverage {h.runtime / 'lua-coverage'}", "--self-test"],
            )
            self.assertIn("skipping Lua coverage ratchet artifact collection", result.stderr)
        finally:
            h.close()

    def test_ratchet_enforces_via_fkst_lua_coverage_json_artifact(self) -> None:
        h = RunShCoverageRatchetHarness()
        try:
            output = h.root / "canonical" / "coverage.json"

            result = h.run_function(output)

            self.assertEqual(result.returncode, 0, result.stderr + result.stdout)
            self.assertEqual(h.check_env_log.read_text(encoding="utf-8"), str(output))
            data = json.loads(output.read_text(encoding="utf-8"))
            self.assertEqual(data["schema"], "fkst.lua.coverage.v1")
            self.assertEqual(data["files"][0]["file"], "packages/example/core.lua")
            self.assertIn("coverable_lines", data["files"][0])
            self.assertNotIn("--covered-json", result.stdout + result.stderr)
        finally:
            h.close()


if __name__ == "__main__":
    unittest.main()
