#!/usr/bin/env python3
"""Tests for the read-only ratchet migration slicer."""

from __future__ import annotations

import importlib.util
import subprocess
import tempfile
import textwrap
import unittest
import sys
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[1]


def load_slicer():
    path = Path(__file__).with_name("ratchet_migration_slicer.py")
    spec = importlib.util.spec_from_file_location("ratchet_migration_slicer", path)
    if spec is None or spec.loader is None:
        raise RuntimeError("could not load ratchet_migration_slicer.py")
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


slicer = load_slicer()


class RatchetMigrationSlicerTest(unittest.TestCase):
    def test_gh_git_allowlist_maps_file_and_head_to_source_line(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            source = root / "packages/example/core/main.lua"
            source.parent.mkdir(parents=True)
            source.write_text(
                textwrap.dedent(
                    """\
                    local function ignored()
                      log.info("gh issue should stay a message")
                    end

                    local function run()
                      exec_sync("gh issue view 42")
                      exec_sync("git status --short")
                    end
                    """
                ),
                encoding="utf-8",
            )
            migration = root / "migration"
            migration.mkdir()
            (migration / "gh-git-adapter.allowlist").write_text(
                textwrap.dedent(
                    """\
                    packages/example/core/main.lua:
                      - gh issue
                      - git status
                    """
                ),
                encoding="utf-8",
            )

            spec = slicer.specs()["891"]
            inventory = slicer.load_gh_git_inventory(root, spec)

            self.assertEqual([site.site_ref() for site in inventory], [
                "packages/example/core/main.lua:6",
                "packages/example/core/main.lua:7",
            ])
            self.assertEqual([site.detail for site in inventory], [
                "command_head: gh issue",
                "command_head: git status",
            ])

    def test_saga_allowlist_maps_department_to_pipeline_line(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            first = root / "packages/example/departments/a/main.lua"
            second = root / "packages/example/departments/b/main.lua"
            first.parent.mkdir(parents=True)
            second.parent.mkdir(parents=True)
            first.write_text(
                textwrap.dedent(
                    """\
                    local M = {}

                    function pipeline(event)
                      return event
                    end
                    """
                ),
                encoding="utf-8",
            )
            second.write_text(
                textwrap.dedent(
                    """\
                    local M = {}
                    pipeline = function(event)
                      return event
                    end
                    """
                ),
                encoding="utf-8",
            )
            migration = root / "migration"
            migration.mkdir()
            (migration / "saga-handler.allowlist").write_text(
                "\n".join([
                    "packages/example/departments/b/main.lua",
                    "packages/example/departments/a/main.lua",
                    "",
                ]),
                encoding="utf-8",
            )

            spec = slicer.specs()["892"]
            inventory = slicer.load_saga_inventory(root, spec)

            self.assertEqual([site.site_ref() for site in inventory], [
                "packages/example/departments/a/main.lua:2",
                "packages/example/departments/b/main.lua:2",
            ])
            self.assertEqual([site.detail for site in inventory], [
                "free_form_pipeline",
                "free_form_pipeline",
            ])

    def test_saga_allowlist_keeps_already_migrated_entry_as_removal_site(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            source = root / "packages/example/departments/a/main.lua"
            source.parent.mkdir(parents=True)
            source.write_text(
                textwrap.dedent(
                    """\
                    local saga = require("std.saga")
                    return saga.department({ spec = {}, handlers = {} })
                    """
                ),
                encoding="utf-8",
            )
            migration = root / "migration"
            migration.mkdir()
            (migration / "saga-handler.allowlist").write_text(
                "packages/example/departments/a/main.lua\n",
                encoding="utf-8",
            )

            spec = slicer.specs()["892"]
            inventory = slicer.load_saga_inventory(root, spec)

            self.assertEqual(len(inventory), 1)
            self.assertEqual(inventory[0].site_ref(), "packages/example/departments/a/main.lua:1")
            self.assertEqual(inventory[0].detail, "stale_allowlist_entry")

    def test_render_child_issue_is_bounded_and_non_emitting(self) -> None:
        spec = slicer.specs()["892"]
        inventory = [
            slicer.InventorySite("packages/example/a.lua", 3, "free_form_pipeline"),
            slicer.InventorySite("packages/example/b.lua", 4, "free_form_pipeline"),
            slicer.InventorySite("packages/example/c.lua", 5, "free_form_pipeline"),
        ]

        body = slicer.render_child_issue(spec, inventory, 2)

        self.assertIn("Dry-run child issue draft. No GitHub state was modified.", body)
        self.assertIn("- parent_issue: #892", body)
        self.assertIn("- migration_kind: allowlist", body)
        self.assertIn("- current_count: 3", body)
        self.assertIn("- target_count: 0", body)
        self.assertIn("- selected_count: 2", body)
        self.assertIn("- packages/example/a.lua:3 (free_form_pipeline)", body)
        self.assertIn("- packages/example/b.lua:4 (free_form_pipeline)", body)
        self.assertNotIn("packages/example/c.lua:5", body)
        self.assertIn("- The allowlist count decreases by exactly 2.", body)

    def test_rejects_paths_that_escape_repo_root(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            with self.assertRaises(ValueError):
                slicer.validated_repo_path(root, "../outside.lua")

    def test_current_repo_parents_print_dry_run_bodies(self) -> None:
        for parent in ("891", "892"):
            result = subprocess.run(
                [
                    "python3",
                    "-B",
                    "scripts/ratchet_migration_slicer.py",
                    parent,
                    "--repo-root",
                    str(REPO_ROOT),
                    "--slice-size",
                    "2",
                ],
                cwd=REPO_ROOT,
                text=True,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                check=False,
            )
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertIn(f"- parent_issue: #{parent}", result.stdout)
            self.assertIn("- migration_kind: allowlist", result.stdout)
            self.assertRegex(result.stdout, r"- selected_count: [02]")
            self.assertIn("## Acceptance Criteria", result.stdout)


if __name__ == "__main__":
    unittest.main()
