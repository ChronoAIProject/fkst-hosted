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


class FakeGithubClient:
    def __init__(self) -> None:
        self.parent = {"number": 979, "state": "OPEN", "comments": []}
        self.search_results: dict[tuple[str, str], list[dict[str, object]]] = {}
        self.created: list[dict[str, object]] = []
        self.comments: list[tuple[int, str]] = []
        self.closed: list[int] = []

    def issue_view(self, repo: str, number: int, fields: str) -> dict[str, object]:
        self.viewed = (repo, number, fields)
        return self.parent

    def issue_search(self, repo: str, state: str, query: str) -> list[dict[str, object]]:
        self.searched = getattr(self, "searched", [])
        self.searched.append((repo, state, query))
        return list(self.search_results.get((state, query), []))

    def issue_comment(self, repo: str, number: int, body: str) -> None:
        self.comments.append((number, body))
        self.parent.setdefault("comments", []).append({
            "author": {"login": "fkst-bot"},
            "body": body,
        })

    def issue_create(self, repo: str, title: str, body: str, labels: list[str]) -> int:
        number = 1200 + len(self.created)
        self.created.append({
            "repo": repo,
            "title": title,
            "body": body,
            "labels": labels,
            "number": number,
        })
        return number

    def issue_close(self, repo: str, number: int) -> None:
        self.closed.append(number)
        self.parent["state"] = "CLOSED"


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

    def test_registered_ratchets_use_live_parent_tracks(self) -> None:
        all_specs = slicer.specs()

        self.assertEqual(all_specs["saga-handler"].parent, "979")
        self.assertEqual(all_specs["code-dedup"].parent, "1018")
        self.assertNotEqual(all_specs["code-dedup"].parent, "1002")

    def test_reconciler_dry_run_reports_one_slice_without_writing(self) -> None:
        spec = slicer.specs()["saga-handler"]
        inventory = [
            slicer.InventorySite("packages/example/a.lua", 3, "free_form_pipeline"),
            slicer.InventorySite("packages/example/b.lua", 4, "free_form_pipeline"),
        ]
        client = FakeGithubClient()

        result = slicer.reconcile_ratchet(
            spec,
            inventory,
            2,
            "owner/repo",
            client,
            env={},
        )

        self.assertEqual(result.action, "would-create-slice")
        self.assertEqual(result.parent_issue, 979)
        self.assertEqual(client.created, [])
        self.assertEqual(client.comments, [])
        self.assertEqual(client.closed, [])

    def test_reconciler_dedups_existing_in_flight_slice(self) -> None:
        spec = slicer.specs()["saga-handler"]
        inventory = [slicer.InventorySite("packages/example/a.lua", 3, "free_form_pipeline")]
        client = FakeGithubClient()
        client.search_results[("open", slicer.ratchet_slice_search_query("saga-handler"))] = [{
            "number": 123,
            "author": {"login": "fkst-bot"},
            "body": '<!-- fkst:ratchet-slice:v1 schema="fkst.ratchet-slice.v1" ratchet="saga-handler" parent="979" dedup="saga-handler/slice/old" fingerprint="old" -->',
        }]

        result = slicer.reconcile_ratchet(
            spec,
            inventory,
            1,
            "owner/repo",
            client,
            env={"FKST_GITHUB_BOT_LOGIN": "fkst-bot"},
        )

        self.assertEqual(result.action, "deduped-in-flight")
        self.assertEqual(result.issue_number, 123)
        self.assertEqual(client.created, [])

    def test_reconciler_dedups_parent_created_marker(self) -> None:
        spec = slicer.specs()["saga-handler"]
        inventory = [slicer.InventorySite("packages/example/a.lua", 3, "free_form_pipeline")]
        doc = slicer.slice_document(spec, inventory, 1)
        client = FakeGithubClient()
        client.parent["comments"] = [{
            "author": {"login": "fkst-bot"},
            "body": slicer.issue_created_marker(str(doc["dedup_key"]), 123),
        }]

        result = slicer.reconcile_ratchet(
            spec,
            inventory,
            1,
            "owner/repo",
            client,
            env={"FKST_GITHUB_BOT_LOGIN": "fkst-bot"},
        )

        self.assertEqual(result.action, "deduped-parent-ledger")
        self.assertEqual(client.created, [])

    def test_reconciler_real_write_uses_intent_marker_and_creates_one_issue(self) -> None:
        spec = slicer.specs()["saga-handler"]
        inventory = [slicer.InventorySite("packages/example/a.lua", 3, "free_form_pipeline")]
        client = FakeGithubClient()

        result = slicer.reconcile_ratchet(
            spec,
            inventory,
            1,
            "owner/repo",
            client,
            env={"FKST_GITHUB_WRITE": "1", "FKST_GITHUB_BOT_LOGIN": "fkst-bot"},
            labels=["fkst-dev:enabled"],
        )

        self.assertEqual(result.action, "created-slice")
        self.assertEqual(len(client.created), 1)
        self.assertEqual(client.created[0]["labels"], ["fkst-dev:enabled"])
        self.assertIn("Machine-filed ratchet slice issue.", str(client.created[0]["body"]))
        self.assertIn("<!-- fkst:github-proxy:issue-create:", str(client.created[0]["body"]))
        self.assertIn("<!-- fkst:ratchet-slice:v1", str(client.created[0]["body"]))
        self.assertEqual(len(client.comments), 2)
        self.assertIn("issue-create-intent:v1", client.comments[0][1])
        self.assertIn("issue-created:v1", client.comments[1][1])

    def test_reconciler_empty_inventory_closes_parent_only_when_write_enabled(self) -> None:
        spec = slicer.specs()["saga-handler"]
        dry_client = FakeGithubClient()

        dry = slicer.reconcile_ratchet(spec, [], 1, "owner/repo", dry_client, env={})

        self.assertEqual(dry.action, "would-close-parent")
        self.assertEqual(dry_client.closed, [])

        real_client = FakeGithubClient()
        real = slicer.reconcile_ratchet(
            spec,
            [],
            1,
            "owner/repo",
            real_client,
            env={"FKST_GITHUB_WRITE": "1", "FKST_GITHUB_BOT_LOGIN": "fkst-bot"},
        )

        self.assertEqual(real.action, "closed-parent")
        self.assertEqual(real_client.closed, [979])

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
