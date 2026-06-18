#!/usr/bin/env python3
"""Tests for the read-only ratchet migration slicer."""

from __future__ import annotations

import importlib.util
import json
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

            spec = slicer.specs()["gh-git-adapter"]
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

            spec = slicer.specs()["saga-handler"]
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
                    local spec = { consumes = { "q" } }
                    return saga.department(spec, { done = done, act = act })
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

            spec = slicer.specs()["saga-handler"]
            inventory = slicer.load_saga_inventory(root, spec)

            self.assertEqual(len(inventory), 1)
            self.assertEqual(inventory[0].site_ref(), "packages/example/departments/a/main.lua:1")
            self.assertEqual(inventory[0].detail, "stale_allowlist_entry")

    def test_render_child_issue_is_bounded_and_non_emitting(self) -> None:
        spec = slicer.specs()["saga-handler"]
        inventory = [
            slicer.InventorySite("packages/example/a.lua", 3, "free_form_pipeline"),
            slicer.InventorySite("packages/example/b.lua", 4, "free_form_pipeline"),
            slicer.InventorySite("packages/example/c.lua", 5, "free_form_pipeline"),
        ]

        body = slicer.render_child_issue(spec, inventory, 2)

        self.assertIn("Dry-run child issue draft. No GitHub state was modified.", body)
        self.assertIn("- parent_issue: #979", body)
        self.assertIn("- ratchet: `saga-handler`", body)
        self.assertIn("- migration_kind: `allowlist`", body)
        self.assertIn("- current_count: 3", body)
        self.assertIn("- target_count: 0", body)
        self.assertIn("- selected_count: 2", body)
        self.assertIn("- `packages/example/a.lua:3` (`free_form_pipeline`)", body)
        self.assertIn("- `packages/example/b.lua:4` (`free_form_pipeline`)", body)
        self.assertNotIn("packages/example/c.lua:5", body)
        self.assertIn("- The allowlist count decreases by exactly 2.", body)

    def test_json_schema_carries_stable_dedup_key_and_sites(self) -> None:
        spec = slicer.specs()["saga-handler"]
        inventory = [
            slicer.InventorySite("packages/example/a.lua", 3, "free_form_pipeline"),
            slicer.InventorySite("packages/example/b.lua", 4, "free_form_pipeline"),
            slicer.InventorySite("packages/example/c.lua", 5, "free_form_pipeline"),
        ]

        doc = slicer.slice_document(spec, inventory, 2)

        self.assertEqual(doc["schema"], "fkst.ratchet-slice.v1")
        self.assertEqual(doc["ratchet"], "saga-handler")
        self.assertEqual(doc["parent_issue"], 979)
        self.assertEqual(doc["selected_count"], 2)
        self.assertEqual(len(doc["sites_fingerprint"]), 16)
        self.assertEqual(doc["dedup_key"], f"saga-handler/slice/{doc['sites_fingerprint']}")
        self.assertEqual(doc["sites"][0]["site_ref"], "packages/example/a.lua:3")
        self.assertEqual(doc["sites"][1]["site_ref"], "packages/example/b.lua:4")

    def test_code_dedup_allowlist_maps_duplicate_group_files(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            first = root / "packages/example/a.lua"
            second = root / "std/b.lua"
            first.parent.mkdir(parents=True)
            second.parent.mkdir(parents=True)
            body = textwrap.dedent(
                """\
                local function repeated(value)
                  local text = tostring(value or "")
                  text = text:gsub("^%s+", ""):gsub("%s+$", "")
                  if text == "" then
                    return "empty"
                  end
                  return text
                end
                """
            )
            first.write_text(body, encoding="utf-8")
            second.write_text(body, encoding="utf-8")
            migration = root / "migration"
            migration.mkdir()
            source_map = {
                "packages/example/a.lua": first.read_text(encoding="utf-8"),
                "std/b.lua": second.read_text(encoding="utf-8"),
            }
            entry = next(iter(slicer.code_dedup.duplicate_groups(source_map)))
            (migration / "code-dedup.allowlist").write_text(entry.allowlist_line() + "\n", encoding="utf-8")

            spec = slicer.specs()["code-dedup"]
            inventory = slicer.load_code_dedup_inventory(root, spec)

            self.assertEqual([site.site_ref() for site in inventory], [
                "packages/example/a.lua:1",
                "std/b.lua:1",
            ])
            self.assertEqual([site.detail for site in inventory], [
                f"duplicate_function: repeated {entry.body_hash}",
                f"duplicate_function: repeated {entry.body_hash}",
            ])

    def test_rejects_paths_that_escape_repo_root(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            with self.assertRaises(ValueError):
                slicer.validated_repo_path(root, "../outside.lua")

    def test_current_repo_parents_print_dry_run_bodies(self) -> None:
        for ratchet in ("gh-git-adapter", "saga-handler", "code-dedup"):
            result = subprocess.run(
                [
                    "python3",
                    "-B",
                    "scripts/ratchet_migration_slicer.py",
                    ratchet,
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
            self.assertIn(f"- ratchet: `{ratchet}`", result.stdout)
            self.assertIn("- migration_kind: `allowlist`", result.stdout)
            self.assertRegex(result.stdout, r"- selected_count: [02]")
            self.assertIn("## Acceptance Criteria", result.stdout)

    def test_current_repo_ratchets_print_json_schema(self) -> None:
        for ratchet in ("saga-handler", "code-dedup"):
            result = subprocess.run(
                [
                    "python3",
                    "-B",
                    "scripts/ratchet_migration_slicer.py",
                    ratchet,
                    "--repo-root",
                    str(REPO_ROOT),
                    "--slice-size",
                    "2",
                    "--json",
                ],
                cwd=REPO_ROOT,
                text=True,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                check=False,
            )
            self.assertEqual(result.returncode, 0, result.stderr)
            doc = json.loads(result.stdout)
            self.assertEqual(doc["schema"], "fkst.ratchet-slice.v1")
            self.assertEqual(doc["ratchet"], ratchet)
            self.assertEqual(doc["dedup_key"], f"{ratchet}/slice/{doc['sites_fingerprint']}")


if __name__ == "__main__":
    unittest.main()
