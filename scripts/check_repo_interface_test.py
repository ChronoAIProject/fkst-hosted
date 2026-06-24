#!/usr/bin/env python3
"""Contract tests for the shared check_repo.py CLI seam."""

from __future__ import annotations

import subprocess
import sys
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
CHECK_REPO = ROOT / "scripts" / "check_repo.py"


class CheckRepoPublishedInterfaceTest(unittest.TestCase):
    def run_check(self, project_root: Path, *extra: str) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            [sys.executable, "-B", str(CHECK_REPO), "--project-root", str(project_root), *extra],
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            check=False,
        )

    def test_external_project_root_uses_host_package_view_and_skips_b_only_ratchets(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            source_package = root / "packages" / "source-board"
            local_package = root / ".fkst" / "local-packages" / "site-board"
            source_package.mkdir(parents=True)
            local_package.mkdir(parents=True)
            for package in (source_package, local_package):
                (package / "core.lua").write_text(
                    'local M = {}\nfunction M.persistence_class() return "stateless_adapter" end\nreturn M\n',
                    encoding="utf-8",
                )

            result = self.run_check(root)

        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("OK: skipped library-B-specific ratchets for external project root:", result.stdout)
        self.assertIn("OK: repository checks passed", result.stdout)
        self.assertNotIn("G-DOGFOOD-BOUNDARY", result.stderr)
        self.assertNotIn("G-LIB-DEP", result.stderr)

    def test_external_project_root_runs_generic_ratchets(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            source_package = root / "packages" / "source-board"
            local_package = root / ".fkst" / "local-packages" / "site-board"
            source_package.mkdir(parents=True)
            local_package.mkdir(parents=True)
            (source_package / "core.lua").write_text("local M = {}\nreturn M\n", encoding="utf-8")
            (local_package / "core.lua").write_text("-- filler\n" * 1001, encoding="utf-8")

            result = self.run_check(root)

        self.assertNotEqual(result.returncode, 0)
        self.assertIn("repository check failed:", result.stderr)
        # persistence_class declaration presence is now enforced by the engine
        # `engine.persistence-class` conformance check (covered in substrate's
        # host_conformance tests), not this Python generic ratchet; G1 below is the
        # witness that generic ratchets still run on an external project root.
        self.assertIn("G1: packages/site-board/core.lua has 1001 lines; limit is 1000", result.stderr)
        self.assertIn("OK: skipped library-B-specific ratchets for external project root:", result.stdout)


if __name__ == "__main__":
    unittest.main()
