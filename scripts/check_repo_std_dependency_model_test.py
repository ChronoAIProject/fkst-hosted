#!/usr/bin/env python3
"""Unit tests for the std dependency-model repository guard."""

from __future__ import annotations

import importlib.util
import sys
import tempfile
import unittest
from pathlib import Path


def load_check_repo():
    path = Path(__file__).with_name("check_repo.py")
    spec = importlib.util.spec_from_file_location("check_repo", path)
    if spec is None or spec.loader is None:
        raise RuntimeError("could not load check_repo.py")
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


check_repo = load_check_repo()


def write(path: Path, source: str) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(source, encoding="utf-8")


class StdDependencyModelGuardTest(unittest.TestCase):
    def run_guard(self, root: Path) -> tuple[list[str], list[str]]:
        violations: list[str] = []
        warnings: list[str] = []
        check_repo.check_std_dependency_model(root, violations, warnings)
        return violations, warnings

    def test_allows_resolved_package_and_std_dependencies_and_reports_usage(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            write(root / "std" / "github.lua", 'local git = require("std.git")\nreturn {}\n')
            write(root / "std" / "git.lua", "return {}\n")
            write(root / "packages" / "example" / "core.lua", 'local github = require("std.github")\n')

            violations, warnings = self.run_guard(root)

        self.assertEqual(violations, [])
        self.assertIn("G-STD-DEP: package example uses std.github", warnings)
        self.assertIn("G-STD-DEP: std internal edge std.github -> std.git", warnings)

    def test_flags_unresolved_std_require_from_package_and_std_source(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            write(root / "std" / "github.lua", 'local missing = require("std.missing")\n')
            write(root / "packages" / "example" / "core.lua", "local missing = require('std.nope.tools')\n")

            violations, _warnings = self.run_guard(root)

        self.assertEqual(len(violations), 2)
        self.assertTrue(all(message.startswith("G-STD-DEP: ") for message in violations))
        self.assertIn("std/github.lua:1 requires unresolved module 'std.missing'", violations[0])
        self.assertIn("packages/example/core.lua:1 requires unresolved module 'std.nope.tools'", violations[1])

    def test_flags_std_requiring_a_package_module(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            write(root / "packages" / "example" / "core.lua", "return {}\n")
            write(root / "std" / "bad.lua", 'local core = require("example.core")\n')

            violations, warnings = self.run_guard(root)

        self.assertEqual(warnings, [])
        self.assertIn(
            'G-STD-DEP: std module std/bad.lua:1 requires non-std module "example.core" '
            "(std must receive resolved values from package-owned wiring)",
            violations,
        )

    def test_flags_std_requiring_bare_package_internal_roots(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            write(root / "packages" / "example" / "core.lua", "return {}\n")
            write(
                root / "std" / "bad.lua",
                'local core = require("core")\nlocal dept = require("departments.x")\n',
            )

            violations, warnings = self.run_guard(root)

        self.assertEqual(warnings, [])
        self.assertIn(
            'G-STD-DEP: std module std/bad.lua:1 requires non-std module "core" '
            "(std must receive resolved values from package-owned wiring)",
            violations,
        )
        self.assertIn(
            'G-STD-DEP: std module std/bad.lua:2 requires non-std module "departments.x" '
            "(std must receive resolved values from package-owned wiring)",
            violations,
        )

    def test_ignores_masked_requires_in_std_comments_and_strings(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            write(
                root / "std" / "ok.lua",
                '-- local missing = require("std.bogus")\nlocal text = "require(\\"core\\")"\nreturn {}\n',
            )

            violations, warnings = self.run_guard(root)

        self.assertEqual(violations, [])
        self.assertEqual(warnings, [])

    def test_resolves_nested_std_init_modules(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            write(root / "std" / "a" / "b" / "init.lua", "return {}\n")
            write(root / "packages" / "example" / "core.lua", 'local nested = require("std.a.b")\n')

            violations, warnings = self.run_guard(root)

        self.assertEqual(violations, [])
        self.assertEqual(warnings, ["G-STD-DEP: package example uses std.a.b"])

    def test_package_scan_excludes_std_symlink_subtree(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            write(root / "std" / "ok.lua", "return {}\n")
            package = root / "packages" / "example"
            write(package / "core.lua", 'local ok = require("std.ok")\n')
            (package / "std").symlink_to("../../std")

            violations, warnings = self.run_guard(root)

        self.assertEqual(violations, [])
        self.assertEqual(warnings, ["G-STD-DEP: package example uses std.ok"])


if __name__ == "__main__":
    unittest.main()
