#!/usr/bin/env python3
"""Tests for the G7 error-class shrink-only ratchet."""

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
error_class = check_repo.check_repo_error_class


class ErrorClassRatchetTest(unittest.TestCase):
    def test_unallowlisted_current_site_fails(self) -> None:
        messages = error_class.ratchet_messages(
            {"packages/example/core.lua:2"},
            set(),
        )

        self.assertEqual(len(messages), 1)
        self.assertIn("not in migration/error-class.allowlist", messages[0])

    def test_allowlisted_site_passes_and_stale_entry_is_ignored(self) -> None:
        current = {"packages/example/core.lua:2"}
        allowlist = {
            "packages/example/core.lua:2",
            "packages/example/core.lua:9",
        }

        self.assertEqual(error_class.ratchet_messages(current, allowlist), [])

    def test_allowlist_growth_relative_to_base_fails(self) -> None:
        messages = error_class.ratchet_messages(
            {"packages/example/core.lua:2"},
            {"packages/example/core.lua:2"},
            base_allowlist=set(),
        )

        self.assertEqual(len(messages), 1)
        self.assertIn("grows migration/error-class.allowlist relative to dev", messages[0])

    def test_load_allowlist_accepts_path_line_with_reason(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "error-class.allowlist"
            path.write_text(
                "# comment\npackages/example/core.lua:2  # existing debt\n\n",
                encoding="utf-8",
            )

            self.assertEqual(
                error_class.load_allowlist(path),
                {"packages/example/core.lua:2"},
            )

    def test_wrapper_loads_allowlist_and_blocks_growth(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            source = root / "packages" / "example" / "core.lua"
            source.parent.mkdir(parents=True)
            source.write_text('error("missing class")\n', encoding="utf-8")
            migration = root / "migration"
            migration.mkdir()
            (migration / "error-class.allowlist").write_text(
                "packages/example/core.lua:1\n",
                encoding="utf-8",
            )

            violations: list[str] = []
            with mock.patch.object(error_class, "allowlist_at_dev_base", return_value=("absent", None)):
                check_repo.check_error_class_prefixes(root, violations)
            self.assertEqual(violations, [])

            (migration / "error-class.allowlist").write_text("", encoding="utf-8")
            with mock.patch.object(error_class, "allowlist_at_dev_base", return_value=("absent", None)):
                check_repo.check_error_class_prefixes(root, violations)

        self.assertEqual(len(violations), 1)
        self.assertTrue(violations[0].startswith("G7: "))
        self.assertIn("packages/example/core.lua:1", violations[0])


if __name__ == "__main__":
    unittest.main()
