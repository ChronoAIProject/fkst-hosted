#!/usr/bin/env python3
"""Unit tests for the positive library dependency-model repository guard."""

from __future__ import annotations

import importlib.util
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock


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


def manifest(path: Path, name: str, deps: list[str], *, public: bool | None = None, allow: list[str] | None = None) -> None:
    visibility = ""
    if public is not None:
        visibility = f"\n[visibility]\npublic = {'true' if public else 'false'}\n"
    if allow is not None:
        quoted = ", ".join(f'"{item}"' for item in allow)
        visibility = f"\n[visibility]\nallow = [{quoted}]\n"
    quoted_deps = ", ".join(f'"{item}"' for item in deps)
    write(
        path,
        f'kind = "library"\nname = "{name}"\n\n[lib_deps]\nlibraries = [{quoted_deps}]\n{visibility}',
    )


def inventory_line(path: str, module: str) -> str:
    return f'{{"path":"{path}","module":"{module}"}}\n'


class LibraryDependencyModelGuardTest(unittest.TestCase):
    def seed_contract(self, root: Path) -> None:
        manifest(root / "libraries" / "contract" / "fkst.toml", "contract", [], public=True)
        for module in ("error_facts", "payload", "source_ref", "strings"):
            write(root / "libraries" / "contract" / f"{module}.lua", "return {}\n")

    def seed_devloop_manifest(self, root: Path, allow: list[str] | None = None) -> None:
        manifest(
            root / "libraries" / "devloop" / "fkst.toml",
            "devloop",
            ["contract", "workflow", "forge"],
            allow=allow
            or [
                "github-devloop",
                "github-devloop-decompose",
                "github-devloop-intake",
                "github-devloop-integration",
                "github-devloop-ops",
                "github-devloop-pr",
                "fkst-substrate-ref-maintainer",
            ],
        )

    def run_guard(self, root: Path) -> tuple[list[str], list[str]]:
        self.seed_contract(root)
        self.seed_devloop_manifest(root)
        violations: list[str] = []
        warnings: list[str] = []
        check_repo.check_std_dependency_model(root, violations, warnings)
        return violations, warnings

    def test_library_publishability_requires_only_contract_public(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            manifest(root / "libraries" / "contract" / "fkst.toml", "contract", [], public=False)
            manifest(root / "libraries" / "workflow" / "fkst.toml", "workflow", ["contract"], public=True)
            self.seed_devloop_manifest(root)
            write(root / "migration" / "devloop-forge-imports.inventory", "")
            with mock.patch.object(
                check_repo.check_repo_std_dependency_model.ratchet_base,
                "file_at_base",
                return_value=("absent", None),
            ):
                violations, _warnings = self.run_guard_without_seed(root)

        self.assertIn("G-LIB-DEP: contract must be the public publishable library", violations)
        self.assertIn("G-LIB-DEP: workflow must not be public/publishable", violations)

    def run_guard_without_seed(self, root: Path) -> tuple[list[str], list[str]]:
        violations: list[str] = []
        warnings: list[str] = []
        check_repo.check_std_dependency_model(root, violations, warnings)
        return violations, warnings


    def test_devloop_visibility_excludes_non_family_packages(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            self.seed_contract(root)
            self.seed_devloop_manifest(root, allow=["github-devloop", "archaudit"])
            write(root / "migration" / "devloop-forge-imports.inventory", "")
            with mock.patch.object(
                check_repo.check_repo_std_dependency_model.ratchet_base,
                "file_at_base",
                return_value=("absent", None),
            ):
                violations, _warnings = self.run_guard_without_seed(root)

        self.assertTrue(any("devloop visibility must list only" in message and "archaudit" in message for message in violations))

    def test_devloop_forge_import_inventory_matches_current_and_legacy_base(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            write(root / "libraries" / "forge" / "github.lua", "return {}\n")
            write(root / "libraries" / "devloop" / "claims.lua", 'local github = require("forge.github")\nreturn {}\n')
            current = inventory_line("libraries/devloop/claims.lua", "forge.github")
            write(root / "migration" / "devloop-forge-imports.inventory", current)
            with mock.patch.object(
                check_repo.check_repo_std_dependency_model.ratchet_base,
                "file_at_base",
                side_effect=[
                    ("absent", None),
                    ("present", inventory_line("libraries/devloop/claims.lua", "std.github")),
                ],
            ):
                violations, _warnings = self.run_guard(root)

        self.assertEqual(violations, [])

    def test_devloop_forge_import_inventory_is_shrink_only(self) -> None:
        current = inventory_line("libraries/devloop/claims.lua", "forge.github")
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            write(root / "libraries" / "forge" / "github.lua", "return {}\n")
            write(root / "libraries" / "devloop" / "claims.lua", 'local github = require("forge.github")\nreturn {}\n')
            write(root / "migration" / "devloop-forge-imports.inventory", current)
            with mock.patch.object(
                check_repo.check_repo_std_dependency_model.ratchet_base,
                "file_at_base",
                return_value=("present", ""),
            ):
                violations, _warnings = self.run_guard(root)

        self.assertEqual(
            violations,
            [
                "G-LIB-DEP: migration/devloop-forge-imports.inventory grows relative to dev: "
                "libraries/devloop/claims.lua forge.github",
            ],
        )

    def test_devloop_forge_import_inventory_accepts_strings_split_path_follow(self) -> None:
        current = inventory_line("libraries/devloop/parsers/misc.lua", "forge.strings")
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            write(root / "libraries" / "forge" / "strings.lua", "return {}\n")
            write(root / "libraries" / "devloop" / "parsers" / "misc.lua", 'local strings = require("forge.strings")\nreturn {}\n')
            write(root / "migration" / "devloop-forge-imports.inventory", current)
            with mock.patch.object(
                check_repo.check_repo_std_dependency_model.ratchet_base,
                "file_at_base",
                return_value=("present", ""),
            ):
                violations, _warnings = self.run_guard(root)

        self.assertEqual(violations, [])


if __name__ == "__main__":
    unittest.main()
