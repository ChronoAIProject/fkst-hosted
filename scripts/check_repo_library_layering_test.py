#!/usr/bin/env python3
"""Tests for the libraries-to-packages layering guard."""

from __future__ import annotations

import importlib.util
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock


def load_module(name: str, path: Path):
    spec = importlib.util.spec_from_file_location(name, path)
    if spec is None or spec.loader is None:
        raise RuntimeError(f"could not load {path}")
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


scripts_dir = Path(__file__).resolve().parent
check_repo = load_module("check_repo", scripts_dir / "check_repo.py")
library_layering = check_repo.check_repo_library_layering


def write(path: Path, source: str) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(source, encoding="utf-8")


class LibraryLayeringGuardTest(unittest.TestCase):
    def run_messages(self, root: Path) -> list[str]:
        with mock.patch.object(library_layering, "allowlist_at_dev_base", return_value=("absent", None)):
            return library_layering.messages(
                root,
                check_repo.package_dirs,
                check_repo.read_text,
                check_repo.rel,
                check_repo.strip_lua_comments_and_strings,
                check_repo.is_unmasked_range,
                enforce_base=True,
            )

    def test_flags_library_require_of_package_only_modules(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            write(root / "packages" / "github-devloop" / "core.lua", "return {}\n")
            write(
                root / "libraries" / "devloop" / "bad.lua",
                'local core = require("core")\nlocal dept = require("departments.foo")\nreturn {}\n',
            )
            write(root / library_layering.ALLOWLIST, "")

            messages = self.run_messages(root)

        self.assertEqual(len(messages), 2)
        self.assertTrue(any("requires package-only module 'core'" in message for message in messages))
        self.assertTrue(any("requires package-only module 'departments.foo'" in message for message in messages))

    def test_allows_workspace_library_requires(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            write(root / "packages" / "github-devloop" / "core.lua", "return {}\n")
            write(
                root / "libraries" / "devloop" / "ok.lua",
                'local contract = require("contract.x")\n'
                'local devloop = require("devloop.y")\n'
                'local forge = require("forge.z")\n'
                "return {}\n",
            )
            write(root / library_layering.ALLOWLIST, "")

            messages = self.run_messages(root)

        self.assertEqual(messages, [])

    def test_real_repo_has_zero_library_layering_violations(self) -> None:
        root = Path(__file__).resolve().parents[1]
        with mock.patch.object(library_layering, "allowlist_at_dev_base", return_value=("absent", None)):
            messages = library_layering.messages(
                root,
                check_repo.package_dirs,
                check_repo.read_text,
                check_repo.rel,
                check_repo.strip_lua_comments_and_strings,
                check_repo.is_unmasked_range,
                enforce_base=True,
            )

        self.assertEqual(messages, [])


if __name__ == "__main__":
    unittest.main()
