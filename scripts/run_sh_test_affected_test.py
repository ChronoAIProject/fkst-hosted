#!/usr/bin/env python3
"""Execution tests for scripts/run.sh test-affected."""

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


class TestAffectedHarness:
    def __init__(self) -> None:
        self.tmp = tempfile.TemporaryDirectory()
        self.root = Path(self.tmp.name) / "repo"
        self.scripts = self.root / "scripts"
        self.log = Path(self.tmp.name) / "runner.log"
        self.runner = Path(self.tmp.name) / "runner.sh"
        self.root.mkdir()
        self.scripts.mkdir()
        for name in (
            "run.sh",
            "bin_bootstrap.sh",
            "host_run.sh",
            "host_entry.sh",
            "composed_manifest.sh",
        ):
            shutil.copy2(REPO_ROOT / "scripts" / name, self.scripts / name)
        test_affected = REPO_ROOT / "scripts" / "test_affected.sh"
        if test_affected.exists():
            shutil.copy2(test_affected, self.scripts / "test_affected.sh")
        self.runner.write_text(
            "#!/bin/sh\n"
            "printf '%s\\n' \"$*\" >> \"$FKST_TEST_AFFECTED_LOG\"\n"
            "exit 0\n",
            encoding="utf-8",
        )
        self.runner.chmod(self.runner.stat().st_mode | stat.S_IXUSR)
        self._init_repo()

    def close(self) -> None:
        self.tmp.cleanup()

    def _git(self, *args: str) -> str:
        result = subprocess.run(
            ["git", *args],
            cwd=self.root,
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            check=False,
        )
        if result.returncode != 0:
            raise AssertionError(result.stderr + result.stdout)
        return result.stdout

    def _write(self, rel: str, text: str) -> None:
        path = self.root / rel
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text, encoding="utf-8")

    def _init_repo(self) -> None:
        self._git("init")
        self._git("config", "user.email", "test@example.com")
        self._git("config", "user.name", "Test Runner")
        self._git("checkout", "-b", "dev")
        self._write("packages/consensus/core.lua", "return {}\n")
        self._write("packages/github-devloop/core.lua", "return {}\n")
        self._write("scripts/helper.sh", "#!/bin/sh\n")
        self._write("README.md", "fixture\n")
        self._git("add", ".")
        self._git("commit", "-m", "initial")
        self._git("checkout", "-b", "integration")
        self._write("libraries/devloop/config.lua", "return {integration = true}\n")
        self._git("add", ".")
        self._git("commit", "-m", "integration ahead")
        self._git("checkout", "-b", "feature")

    def run(self, with_branch_env: bool = True) -> subprocess.CompletedProcess[str]:
        env = os.environ.copy()
        # Scope derives from the worktree's own uncommitted edits, so these env
        # vars must NOT be required; spawned implement/fix codex environments do
        # not carry them. Drop them to assert env-independence (with_branch_env=False).
        env.pop("FKST_DEVLOOP_UPSTREAM_BRANCH", None)
        env.pop("FKST_DEVLOOP_INTEGRATION_BRANCH", None)
        if with_branch_env:
            env["FKST_DEVLOOP_UPSTREAM_BRANCH"] = "dev"
            env["FKST_DEVLOOP_INTEGRATION_BRANCH"] = "integration"
        env["FKST_TEST_AFFECTED_RUNNER"] = str(self.runner)
        env["FKST_TEST_AFFECTED_LOG"] = str(self.log)
        return subprocess.run(
            ["/bin/bash", "scripts/run.sh", "test-affected"],
            cwd=self.root,
            env=env,
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            check=False,
        )

    def runner_args(self) -> list[str]:
        if not self.log.exists():
            return []
        return self.log.read_text(encoding="utf-8").splitlines()


class RunShTestAffectedTest(unittest.TestCase):
    def test_scopes_to_uncommitted_changed_package(self) -> None:
        h = TestAffectedHarness()
        try:
            h._write("packages/github-devloop/core.lua", "return {changed = true}\n")

            result = h.run()

            self.assertEqual(result.returncode, 0, result.stderr + result.stdout)
            self.assertEqual(h.runner_args(), ["test github-devloop"])
        finally:
            h.close()

    def test_works_without_integration_branch_env(self) -> None:
        # Regression guard (#1619 follow-up): the spawned implement/fix codex
        # environment does NOT carry FKST_DEVLOOP_INTEGRATION_BRANCH. The earlier
        # base-ref derivation fail-closed here, breaking every implement. Scope
        # must derive purely from the worktree's uncommitted edits.
        h = TestAffectedHarness()
        try:
            h._write("packages/github-devloop/core.lua", "return {changed = true}\n")

            result = h.run(with_branch_env=False)

            self.assertEqual(result.returncode, 0, result.stderr + result.stdout)
            self.assertEqual(h.runner_args(), ["test github-devloop"])
        finally:
            h.close()

    def test_committed_only_changes_fall_back_to_full(self) -> None:
        # Codex verifies before committing, so its edits are uncommitted at verify
        # time. If nothing is uncommitted (e.g. already committed), fall back to the
        # full suite rather than silently testing nothing.
        h = TestAffectedHarness()
        try:
            h._write("packages/github-devloop/core.lua", "return {committed = true}\n")
            h._git("add", ".")
            h._git("commit", "-m", "committed change")

            result = h.run()

            self.assertEqual(result.returncode, 0, result.stderr + result.stdout)
            self.assertEqual(h.runner_args(), ["test"])
        finally:
            h.close()

    def test_runs_full_for_broad_paths(self) -> None:
        broad_paths = (
            "libraries/devloop/extra.lua",
            "scripts/helper.sh",
            ".github/workflows/ci.yml",
            "fkst.workspace.toml",
        )
        for rel in broad_paths:
            h = TestAffectedHarness()
            try:
                h._write(rel, "changed\n")

                result = h.run()

                self.assertEqual(result.returncode, 0, rel + "\n" + result.stderr + result.stdout)
                self.assertEqual(h.runner_args(), ["test"], rel)
            finally:
                h.close()

    def test_runs_each_changed_package(self) -> None:
        h = TestAffectedHarness()
        try:
            h._write("packages/consensus/core.lua", "return {changed = true}\n")
            h._write("packages/github-devloop/core.lua", "return {changed = true}\n")

            result = h.run()

            self.assertEqual(result.returncode, 0, result.stderr + result.stdout)
            self.assertEqual(h.runner_args(), ["test consensus", "test github-devloop"])
        finally:
            h.close()

    def test_untracked_new_package_file_is_counted(self) -> None:
        h = TestAffectedHarness()
        try:
            h._write("packages/github-devloop/new_module.lua", "return {}\n")

            result = h.run()

            self.assertEqual(result.returncode, 0, result.stderr + result.stdout)
            self.assertEqual(h.runner_args(), ["test github-devloop"])
        finally:
            h.close()


if __name__ == "__main__":
    unittest.main()
