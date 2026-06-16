#!/usr/bin/env python3
"""Tests for fkst runtime layout repository guard."""

from __future__ import annotations

import subprocess
import tempfile
import unittest
from pathlib import Path

import check_repo_fkst_layout


class FkstLayoutGuardTest(unittest.TestCase):
    def make_repo(self) -> Path:
        root = Path(tempfile.mkdtemp(prefix="fkst-layout-test-")) / "repo"
        root.mkdir()
        subprocess.run(["git", "init"], cwd=root, check=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        (root / ".fkst").mkdir()
        (root / ".fkst" / "substrate-ref").write_text("dev\n", encoding="utf-8")
        (root / ".fkst" / "env.example").write_text("BIN=/path/to/fkst-framework\n", encoding="utf-8")
        (root / ".gitignore").write_text(
            "# Local, machine-specific config. Tracked template lives in .fkst/env.example.\n"
            "/.fkst/packages\n"
            "/.fkst/local-packages\n"
            "/.fkst/run/\n"
            "/.fkst/env\n",
            encoding="utf-8",
        )
        subprocess.run(
            ["git", "add", ".gitignore", ".fkst/substrate-ref", ".fkst/env.example"],
            cwd=root,
            check=True,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )
        return root

    def violations(self, root: Path) -> list[str]:
        return check_repo_fkst_layout.check_layout(root)

    def assertViolationContains(self, root: Path, expected: str) -> None:
        violations = self.violations(root)
        self.assertTrue(
            any(expected in violation for violation in violations),
            f"expected violation containing {expected!r}, got {violations!r}",
        )

    def test_clean_repo_passes(self) -> None:
        root = self.make_repo()

        self.assertEqual(self.violations(root), [])

    def test_tracked_runtime_package_dir_fails(self) -> None:
        root = self.make_repo()
        tracked = root / ".fkst" / "packages" / "external" / "core.lua"
        tracked.parent.mkdir(parents=True)
        tracked.write_text("return {}\n", encoding="utf-8")
        subprocess.run(["git", "add", "-f", ".fkst/packages/external/core.lua"], cwd=root, check=True)

        self.assertViolationContains(root, ".fkst/packages must be runtime-only")

    def test_tracked_runtime_path_pending_removal_passes(self) -> None:
        root = self.make_repo()
        tracked = root / ".fkst" / "packages"
        tracked.symlink_to("../packages")
        subprocess.run(["git", "add", "-f", ".fkst/packages"], cwd=root, check=True)
        tracked.unlink()

        self.assertEqual(self.violations(root), [])

    def test_tracked_local_packages_fails(self) -> None:
        root = self.make_repo()
        tracked = root / ".fkst" / "local-packages"
        tracked.symlink_to("../packages")
        subprocess.run(["git", "add", "-f", ".fkst/local-packages"], cwd=root, check=True)

        self.assertViolationContains(root, ".fkst/local-packages must be runtime-only")

    def test_tracked_run_path_fails(self) -> None:
        root = self.make_repo()
        tracked = root / ".fkst" / "run" / "runtime" / "mark"
        tracked.parent.mkdir(parents=True)
        tracked.write_text("runtime\n", encoding="utf-8")
        subprocess.run(["git", "add", "-f", ".fkst/run/runtime/mark"], cwd=root, check=True)

        self.assertViolationContains(root, ".fkst/run/ must be runtime-only")

    def test_root_substrate_ref_fails(self) -> None:
        root = self.make_repo()
        (root / ".fkst-substrate-ref").write_text("dev\n", encoding="utf-8")

        self.assertViolationContains(root, "root .fkst-substrate-ref is forbidden")

    def test_tracked_legacy_runtime_path_fails(self) -> None:
        root = self.make_repo()
        tracked = root / ".fkst" / "runtime" / "mark"
        tracked.parent.mkdir(parents=True)
        tracked.write_text("runtime\n", encoding="utf-8")
        subprocess.run(["git", "add", "-f", ".fkst/runtime/mark"], cwd=root, check=True)

        self.assertViolationContains(root, "legacy generated path is tracked")

    def test_gitignore_blanket_fkst_fails(self) -> None:
        root = self.make_repo()
        (root / ".gitignore").write_text(
            "# Local, machine-specific config. Tracked template lives in .fkst/env.example.\n"
            "/.fkst/\n"
            "/.fkst/packages\n"
            "/.fkst/local-packages\n"
            "/.fkst/run/\n"
            "/.fkst/env\n",
            encoding="utf-8",
        )

        self.assertViolationContains(root, "must not blanket-ignore /.fkst/")

    def test_gitignore_missing_required_line_fails(self) -> None:
        root = self.make_repo()
        (root / ".gitignore").write_text(
            "# Local, machine-specific config. Tracked template lives in .fkst/env.example.\n"
            "/.fkst/packages\n"
            "/.fkst/local-packages\n"
            "/.fkst/env\n",
            encoding="utf-8",
        )

        self.assertViolationContains(root, "missing required .gitignore line: /.fkst/run/")

    def test_missing_substrate_ref_fails(self) -> None:
        root = self.make_repo()
        (root / ".fkst" / "substrate-ref").unlink()
        subprocess.run(["git", "rm", "--cached", ".fkst/substrate-ref"], cwd=root, check=True, stdout=subprocess.DEVNULL)

        self.assertViolationContains(root, "missing .fkst/substrate-ref")


if __name__ == "__main__":
    unittest.main()
