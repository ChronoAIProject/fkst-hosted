#!/usr/bin/env python3
"""Behavior tests for scripts/run.sh doctor."""

from __future__ import annotations

import os
import shutil
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


class DoctorHarness:
    def __init__(self) -> None:
        self.tmp = tempfile.TemporaryDirectory()
        self.root = Path(self.tmp.name) / "repo"
        shutil.copytree(REPO_ROOT, self.root, ignore=shutil.ignore_patterns(".git"))
        self.fake_bin = Path(self.tmp.name) / "fake-bin"
        self.fake_bin.mkdir()
        self.framework = Path(self.tmp.name) / "fkst-framework"
        write_executable(
            self.framework,
            textwrap.dedent(
                """\
                #!/bin/sh
                if [ "$1" = "--self-test" ]; then
                  exit 0
                fi
                if [ "$1" = "run" ]; then
                  echo "fkst-framework $*"
                  exit 0
                fi
                echo "fkst-framework $*" >&2
                exit 1
                """
            ),
        )
        self.env = {
            "PATH": str(self.fake_bin),
            "BIN": str(self.framework),
            "HOME": str(Path(self.tmp.name) / "home"),
        }
        self._install_default_tools()

    def close(self) -> None:
        self.tmp.cleanup()

    def _install_default_tools(self) -> None:
        write_executable(self.fake_bin / "git", "#!/bin/sh\necho 'git version test'\n")
        write_executable(self.fake_bin / "cargo", "#!/bin/sh\necho 'cargo 1.0.0'\n")
        write_executable(self.fake_bin / "rustc", "#!/bin/sh\necho 'rustc 1.0.0'\n")
        write_executable(self.fake_bin / "codex", "#!/bin/sh\necho 'codex test'\n")
        write_executable(
            self.fake_bin / "gh",
            textwrap.dedent(
                """\
                #!/bin/sh
                if [ "$1" = "auth" ] && [ "$2" = "status" ]; then
                  exit 0
                fi
                if [ "$1" = "--version" ]; then
                  echo 'gh version test'
                  exit 0
                fi
                echo 'gh version test'
                exit 0
                """
            ),
        )
        for tool in ["head", "dirname", "pwd", "grep", "tail", "cut", "sed", "basename", "mkdir", "ln"]:
            path = shutil.which(tool)
            if path is None:
                raise RuntimeError(f"required test tool missing: {tool}")
            write_executable(self.fake_bin / tool, f"#!/bin/sh\n{path} \"$@\"\n")

    def run_doctor(self, *args: str) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            ["/bin/bash", "scripts/run.sh", "doctor", *args],
            cwd=self.root,
            env=self.env,
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            check=False,
        )


class DoctorScriptTest(unittest.TestCase):
    def test_complete_environment_exits_zero(self) -> None:
        h = DoctorHarness()
        try:
            result = h.run_doctor()
            self.assertEqual(result.returncode, 0, result.stderr + result.stdout)
            self.assertIn("DOCTOR git ok", result.stdout)
            self.assertIn("DOCTOR cargo ok", result.stdout)
            self.assertIn("DOCTOR rustc ok", result.stdout)
            self.assertIn("DOCTOR bin ok", result.stdout)
            self.assertIn("DOCTOR bin-self-test ok", result.stdout)
            self.assertIn("DOCTOR codex ok", result.stdout)
            self.assertIn("DOCTOR gh ok", result.stdout)
            self.assertIn("DOCTOR gh-auth ok status=authenticated", result.stdout)
        finally:
            h.close()

    def test_missing_codex_is_hard_failure_with_install_hint(self) -> None:
        h = DoctorHarness()
        try:
            (h.fake_bin / "codex").unlink()
            result = h.run_doctor()
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("DOCTOR codex missing hint=npm install -g @openai/codex", result.stdout)
        finally:
            h.close()

    def test_gh_auth_failure_is_hard_failure_with_login_hint(self) -> None:
        h = DoctorHarness()
        try:
            write_executable(
                h.fake_bin / "gh",
                textwrap.dedent(
                    """\
                    #!/bin/sh
                    if [ "$1" = "auth" ] && [ "$2" = "status" ]; then
                      exit 1
                    fi
                    if [ "$1" = "--version" ]; then
                      echo 'gh version test'
                      exit 0
                    fi
                    echo 'gh version test'
                    exit 0
                    """
                ),
            )
            result = h.run_doctor()
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("DOCTOR gh ok", result.stdout)
            self.assertIn("DOCTOR gh-auth missing hint=gh auth login", result.stdout)
        finally:
            h.close()

    def test_tool_version_failure_is_hard_failure(self) -> None:
        h = DoctorHarness()
        try:
            write_executable(h.fake_bin / "cargo", "#!/bin/sh\necho 'cargo broken'\nexit 1\n")
            result = h.run_doctor()
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("DOCTOR cargo missing", result.stdout)
            self.assertIn("detail=cargo broken", result.stdout)
        finally:
            h.close()

    def test_optional_env_missing_does_not_make_doctor_fail(self) -> None:
        h = DoctorHarness()
        try:
            result = h.run_doctor()
            self.assertEqual(result.returncode, 0, result.stderr + result.stdout)
            self.assertIn(
                "DOCTOR env.FKST_GITHUB_REPO missing hint=optional host fact is unset",
                result.stdout,
            )
        finally:
            h.close()

    def test_doctor_does_not_expose_install_option(self) -> None:
        source = (REPO_ROOT / "scripts" / "doctor.sh").read_text(encoding="utf-8")
        self.assertNotIn("--install", source)

    def test_doctor_reuses_shared_bin_resolution_contract(self) -> None:
        source = (REPO_ROOT / "scripts" / "doctor.sh").read_text(encoding="utf-8")
        self.assertIn('resolve_bin_contract "$ROOT" "readonly"', source)
        self.assertNotIn("doctor_resolve_bin", source)
        self.assertNotIn("command -v fkst-framework", source)
        self.assertNotIn("../fkst-substrate/target/debug/fkst-framework", source)

    def test_saga_doctor_runs_from_ops_package(self) -> None:
        h = DoctorHarness()
        try:
            result = h.run_doctor("github-devloop-ops")
            self.assertEqual(result.returncode, 0, result.stderr + result.stdout)
            self.assertIn("packages/github-devloop-ops/departments/doctor/main.lua", result.stdout)
            self.assertIn("--owner-namespace github-devloop-ops", result.stdout)
            self.assertIn('"queue":"devloop_doctor_tick"', result.stdout)
        finally:
            h.close()

    def test_saga_doctor_rejects_old_devloop_package_name(self) -> None:
        h = DoctorHarness()
        try:
            result = h.run_doctor("github-devloop")
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("usage: scripts/run.sh doctor", result.stderr)
            self.assertNotIn("packages/github-devloop-ops/departments/doctor/main.lua", result.stdout)
        finally:
            h.close()

    def test_saga_doctor_running_alias_accepts_ops_package(self) -> None:
        h = DoctorHarness()
        try:
            result = h.run_doctor("--running", "github-devloop-ops")
            self.assertEqual(result.returncode, 0, result.stderr + result.stdout)
            self.assertIn("packages/github-devloop-ops/departments/doctor/main.lua", result.stdout)
        finally:
            h.close()

    def test_saga_doctor_running_alias_rejects_old_devloop_package_name(self) -> None:
        h = DoctorHarness()
        try:
            result = h.run_doctor("--running", "github-devloop")
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("usage: scripts/run.sh doctor", result.stderr)
            self.assertNotIn("packages/github-devloop-ops/departments/doctor/main.lua", result.stdout)
        finally:
            h.close()


if __name__ == "__main__":
    unittest.main()
