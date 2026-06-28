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
from datetime import datetime, timedelta, timezone
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[1]
SLICER_PATH = REPO_ROOT / "packages" / "github-ratchet-migration-slicer" / "tools" / "ratchet_migration_slicer.py"


def load_slicer():
    sys.path.insert(0, str(SLICER_PATH.parent))
    spec = importlib.util.spec_from_file_location("ratchet_migration_slicer", SLICER_PATH)
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
        self.views: dict[int, dict[str, object]] = {}
        self.search_results: dict[tuple[str, str], list[dict[str, object]]] = {}
        self.created: list[dict[str, object]] = []
        self.comments: list[tuple[int, str]] = []
        self.closed: list[int] = []

    def issue_view(self, repo: str, number: int, fields: str) -> dict[str, object]:
        self.viewed = (repo, number, fields)
        if number in self.views:
            return self.views[number]
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
                    local saga = require("workflow.saga")
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
        self.assertIn(
            "- The allowlist count decreases only for listed entries still present in `migration/saga-handler.allowlist` "
            "and may be unchanged only when the slice is already converged.",
            body,
        )
        self.assertNotIn("- The allowlist count decreases by exactly 2.", body)

    def test_render_child_issue_specifies_already_converged_noop(self) -> None:
        spec = slicer.specs()["saga-handler"]
        inventory = [
            slicer.InventorySite("packages/example/a.lua", 3, "free_form_pipeline"),
        ]

        body = slicer.render_child_issue(spec, inventory, 1)

        self.assertIn(
            "- If every listed site is already migrated and every corresponding allowlist entry is already absent, "
            "treat the slice as already converged and make no source changes.",
            body,
        )

    def test_saga_handler_child_issue_defines_single_flight_dedup_contract(self) -> None:
        spec = slicer.specs()["saga-handler"]
        inventory = [
            slicer.InventorySite("packages/example/a.lua", 3, "free_form_pipeline"),
        ]

        body = slicer.render_child_issue(spec, inventory, 1)

        self.assertIn("## Allowlist Contract", body)
        self.assertIn(
            "- A `saga-handler` slice is single-flight by stable `dedup_key`: "
            "at most one live issue or PR surface may own the same `dedup_key`.",
            body,
        )
        self.assertIn(
            "- Before opening or implementing a duplicate slice, prove the prior surface is stale, cancelled, invalid, "
            "or explicitly waived as a duplicate run.",
            body,
        )
        self.assertIn(
            "- If the same `dedup_key` is already live without that proof, "
            "treat the slice as in-flight and make no source changes.",
            body,
        )

    def test_current_repo_d297889b91a40d50_slice_is_already_converged(self) -> None:
        spec = slicer.specs()["saga-handler"]
        inventory = slicer.load_saga_inventory(REPO_ROOT, spec)
        live_paths = {site.path for site in inventory}
        expected_paths = {
            "packages/github-devloop-integration/departments/pr_freshness_scan/main.lua",
            "packages/github-devloop/departments/reconcile/main.lua",
            "packages/github-devloop-pr/departments/review_loop/main.lua",
        }
        allowlist_entries = {
            line.strip()
            for line in (REPO_ROOT / spec.allowlist_path).read_text(encoding="utf-8").splitlines()
            if line.strip() and not line.lstrip().startswith("#")
        }
        expected_entry_keys = {
            "ef2a7b5c6d87abc016a11fa04fd1a04e55ae826259967175c363da002e734acc",
            "a5d78c76cb30e3b341a1ddba5101870076cde9369ec7e458b97af6e4e847effe",
            "1d095528477f7caefe22e45bc119725fdfeacd7982ec534a5f9e383d51073192",
        }
        computed_entry_keys = {
            slicer.entry_key(spec.allowlist_path, slicer.InventorySite(path, 1, "already_migrated", path))
            for path in expected_paths
        }

        self.assertTrue(expected_paths.isdisjoint(live_paths))
        self.assertTrue(expected_paths.isdisjoint(allowlist_entries))
        self.assertEqual(computed_entry_keys, expected_entry_keys)
        self.assertTrue(expected_entry_keys.isdisjoint(allowlist_entries))
        for path in expected_paths:
            source = (REPO_ROOT / path).read_text(encoding="utf-8")
            self.assertIn("return saga.department(spec,", source)
            self.assertIsNone(slicer.FREE_FORM_PIPELINE_RE.search(slicer.strip_lua_comments_and_strings(source)))

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
        self.assertEqual(doc["sites"][0]["allowlist_entry"], "packages/example/a.lua|free_form_pipeline")
        self.assertEqual(len(doc["sites"][0]["entry_key"]), 64)
        self.assertEqual(doc["sites"][1]["site_ref"], "packages/example/b.lua:4")

    def test_registered_ratchets_use_live_parent_tracks(self) -> None:
        all_specs = slicer.specs()

        self.assertEqual(all_specs["saga-handler"].parent, "979")
        self.assertEqual(all_specs["code-dedup"].parent, "1018")
        self.assertNotEqual(all_specs["code-dedup"].parent, "1002")

    def test_controller_plan_excludes_parent_issue_and_wraps_next_slice(self) -> None:
        spec = slicer.specs()["saga-handler"]
        inventory = [slicer.InventorySite("packages/example/a.lua", 3, "free_form_pipeline")]

        doc = slicer.controller_plan(spec, inventory, 1)

        self.assertEqual(doc["schema_version"], "fkst.ratchet-slice.v1")
        self.assertEqual(doc["ratchet"], "saga-handler")
        self.assertEqual(doc["allowlist_path"], "migration/saga-handler.allowlist")
        self.assertEqual(doc["remaining_count"], 1)
        self.assertEqual(doc["slice_size"], 1)
        self.assertEqual(doc["status"], "slice_available")
        self.assertNotIn("parent_issue", doc)
        self.assertRegex(doc["next_slice"]["dedup_key"], r"^saga-handler/slice/[0-9a-f]{16}$")
        self.assertRegex(doc["next_slice"]["sites"][0]["entry_key"], r"^[0-9a-f]{64}$")
        self.assertIn("Machine-filed ratchet slice issue.", doc["next_slice"]["body"])
        self.assertIn('entry_key="', doc["next_slice"]["body"])
        self.assertIn('allowlist_path="migration/saga-handler.allowlist"', doc["next_slice"]["body"])
        self.assertIn('generation="1"', doc["next_slice"]["body"])
        self.assertIn('coord_ref="refs/fkst/migration-slices/', doc["next_slice"]["body"])
        self.assertEqual(doc["next_slice"]["labels"], ["fkst-dev:enabled"])

    def test_controller_plan_empty_inventory_has_no_next_slice(self) -> None:
        spec = slicer.specs()["saga-handler"]

        doc = slicer.controller_plan(spec, [], 1)

        self.assertEqual(doc["status"], "inventory_empty")
        self.assertEqual(doc["remaining_count"], 0)
        self.assertIsNone(doc["next_slice"])

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
        doc = slicer.slice_document(spec, inventory, 1)
        client = FakeGithubClient()
        client.search_results[("open", slicer.ratchet_slice_search_query("saga-handler"))] = [{
            "number": 123,
            "author": {"login": "fkst-bot"},
            "body": '<!-- fkst:ratchet-slice:v1 schema="fkst.ratchet-slice.v1" ratchet="saga-handler" parent="979" dedup="saga-handler/slice/old" fingerprint="old" entries="'
            + str(doc["sites"][0]["entry_key"])
            + '" -->',
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

    def test_reconciler_ignores_in_flight_slice_for_different_entry(self) -> None:
        spec = slicer.specs()["saga-handler"]
        inventory = [slicer.InventorySite("packages/example/a.lua", 3, "free_form_pipeline")]
        client = FakeGithubClient()
        client.search_results[("open", slicer.ratchet_slice_search_query("saga-handler"))] = [{
            "number": 123,
            "author": {"login": "fkst-bot"},
            "body": '<!-- fkst:ratchet-slice:v1 schema="fkst.ratchet-slice.v1" ratchet="saga-handler" parent="979" dedup="saga-handler/slice/old" fingerprint="old" entries="0000000000000000" -->',
        }]

        result = slicer.reconcile_ratchet(
            spec,
            inventory,
            1,
            "owner/repo",
            client,
            env={"FKST_GITHUB_BOT_LOGIN": "fkst-bot"},
        )

        self.assertEqual(result.action, "would-create-slice")
        self.assertEqual(client.created, [])

    def test_reconciler_dedups_legacy_in_flight_marker_without_entries(self) -> None:
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

    def test_reconciler_dedups_parent_created_marker_only_when_prior_slice_is_open(self) -> None:
        spec = slicer.specs()["saga-handler"]
        inventory = [slicer.InventorySite("packages/example/a.lua", 3, "free_form_pipeline")]
        doc = slicer.slice_document(spec, inventory, 1)
        client = FakeGithubClient()
        client.parent["comments"] = [{
            "author": {"login": "fkst-bot"},
            "body": slicer.issue_created_marker(str(doc["dedup_key"]), 123),
        }]
        client.views[123] = {
            "number": 123,
            "state": "OPEN",
            "author": {"login": "fkst-bot"},
            "body": str(slicer.render_reconciled_issue_body(spec, inventory, 1)),
        }

        result = slicer.reconcile_ratchet(
            spec,
            inventory,
            1,
            "owner/repo",
            client,
            env={"FKST_GITHUB_BOT_LOGIN": "fkst-bot"},
        )

        self.assertEqual(result.action, "deduped-parent-ledger")
        self.assertEqual(result.issue_number, 123)
        self.assertEqual(client.created, [])

    def test_reconciler_recreates_parent_ledger_slice_when_prior_slice_is_closed(self) -> None:
        spec = slicer.specs()["saga-handler"]
        inventory = [slicer.InventorySite("packages/example/a.lua", 3, "free_form_pipeline")]
        doc = slicer.slice_document(spec, inventory, 1)
        client = FakeGithubClient()
        client.parent["comments"] = [{
            "author": {"login": "fkst-bot"},
            "body": slicer.issue_created_marker(str(doc["dedup_key"]), 123),
        }]
        client.views[123] = {
            "number": 123,
            "state": "CLOSED",
            "author": {"login": "fkst-bot"},
            "body": str(slicer.render_reconciled_issue_body(spec, inventory, 1)),
        }

        result = slicer.reconcile_ratchet(
            spec,
            inventory,
            1,
            "owner/repo",
            client,
            env={"FKST_GITHUB_BOT_LOGIN": "fkst-bot"},
        )

        self.assertEqual(result.action, "would-create-slice")
        self.assertEqual(client.created, [])

    def test_reconciler_dedups_parent_ledger_later_open_retry_after_closed_child(self) -> None:
        spec = slicer.specs()["saga-handler"]
        inventory = [slicer.InventorySite("packages/example/a.lua", 3, "free_form_pipeline")]
        doc = slicer.slice_document(spec, inventory, 1)
        client = FakeGithubClient()
        client.parent["comments"] = [
            {
                "author": {"login": "fkst-bot"},
                "body": slicer.issue_created_marker(str(doc["dedup_key"]), 123),
            },
            {
                "author": {"login": "fkst-bot"},
                "body": slicer.issue_created_marker(str(doc["dedup_key"]), 124),
            },
        ]
        client.views[123] = {
            "number": 123,
            "state": "CLOSED",
            "author": {"login": "fkst-bot"},
            "body": str(slicer.render_reconciled_issue_body(spec, inventory, 1)),
        }
        client.views[124] = {
            "number": 124,
            "state": "OPEN",
            "author": {"login": "fkst-bot"},
            "body": str(slicer.render_reconciled_issue_body(spec, inventory, 1)),
        }

        result = slicer.reconcile_ratchet(
            spec,
            inventory,
            1,
            "owner/repo",
            client,
            env={"FKST_GITHUB_BOT_LOGIN": "fkst-bot"},
        )

        self.assertEqual(result.action, "deduped-parent-ledger")
        self.assertEqual(result.issue_number, 124)
        self.assertEqual(client.created, [])
        self.assertEqual(getattr(client, "searched", []), [])

    def test_reconciler_dedups_parent_created_marker_when_unknown_issue_is_recent(self) -> None:
        spec = slicer.specs()["saga-handler"]
        inventory = [slicer.InventorySite("packages/example/a.lua", 3, "free_form_pipeline")]
        doc = slicer.slice_document(spec, inventory, 1)
        client = FakeGithubClient()
        client.parent["comments"] = [{
            "author": {"login": "fkst-bot"},
            "body": slicer.issue_created_marker(str(doc["dedup_key"]), None),
            "createdAt": datetime.now(timezone.utc).isoformat(),
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
        self.assertEqual(getattr(client, "searched", []), [])

    def test_reconciler_retries_parent_created_marker_when_unknown_issue_is_stale(self) -> None:
        spec = slicer.specs()["saga-handler"]
        inventory = [slicer.InventorySite("packages/example/a.lua", 3, "free_form_pipeline")]
        doc = slicer.slice_document(spec, inventory, 1)
        client = FakeGithubClient()
        client.parent["comments"] = [{
            "author": {"login": "fkst-bot"},
            "body": slicer.issue_created_marker(str(doc["dedup_key"]), None),
            "createdAt": (datetime.now(timezone.utc) - timedelta(minutes=20)).isoformat(),
        }]

        result = slicer.reconcile_ratchet(
            spec,
            inventory,
            1,
            "owner/repo",
            client,
            env={"FKST_GITHUB_BOT_LOGIN": "fkst-bot"},
        )

        self.assertEqual(result.action, "would-create-slice")
        self.assertEqual(client.created, [])
        self.assertIn(("owner/repo", "open", slicer.ratchet_slice_search_query("saga-handler")), getattr(client, "searched", []))

    def test_reconciler_dedups_parent_created_marker_when_child_is_untrusted(self) -> None:
        spec = slicer.specs()["saga-handler"]
        inventory = [slicer.InventorySite("packages/example/a.lua", 3, "free_form_pipeline")]
        doc = slicer.slice_document(spec, inventory, 1)
        client = FakeGithubClient()
        client.parent["comments"] = [{
            "author": {"login": "fkst-bot"},
            "body": slicer.issue_created_marker(str(doc["dedup_key"]), 123),
        }]
        client.views[123] = {
            "number": 123,
            "state": "CLOSED",
            "author": {"login": "unknown-user"},
            "body": str(slicer.render_reconciled_issue_body(spec, inventory, 1)),
        }

        result = slicer.reconcile_ratchet(
            spec,
            inventory,
            1,
            "owner/repo",
            client,
            env={"FKST_GITHUB_BOT_LOGIN": "fkst-bot"},
        )

        self.assertEqual(result.action, "deduped-parent-ledger")
        self.assertEqual(result.issue_number, 123)
        self.assertEqual(client.created, [])
        self.assertEqual(getattr(client, "searched", []), [])

    def test_reconciler_open_search_does_not_tombstone_closed_existing_slice(self) -> None:
        spec = slicer.specs()["saga-handler"]
        inventory = [slicer.InventorySite("packages/example/a.lua", 3, "free_form_pipeline")]
        doc = slicer.slice_document(spec, inventory, 1)
        client = FakeGithubClient()
        exact_marker = slicer.issue_create_marker(str(doc["dedup_key"]))
        client.search_results[("all", exact_marker)] = [{
            "number": 123,
            "state": "CLOSED",
            "author": {"login": "fkst-bot"},
            "body": exact_marker,
        }]

        result = slicer.reconcile_ratchet(
            spec,
            inventory,
            1,
            "owner/repo",
            client,
            env={"FKST_GITHUB_BOT_LOGIN": "fkst-bot"},
        )

        self.assertEqual(result.action, "would-create-slice")
        self.assertIn(("owner/repo", "open", exact_marker), getattr(client, "searched", []))
        self.assertNotIn(("owner/repo", "all", exact_marker), getattr(client, "searched", []))

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
        self.assertIn('entries="', str(client.created[0]["body"]))
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
            second = root / "libraries/forge/b.lua"
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
                "libraries/forge/b.lua": second.read_text(encoding="utf-8"),
            }
            entry = next(iter(slicer.code_dedup.duplicate_groups(source_map)))
            (migration / "code-dedup.allowlist").write_text(entry.allowlist_line() + "\n", encoding="utf-8")

            spec = slicer.specs()["code-dedup"]
            inventory = slicer.load_code_dedup_inventory(root, spec)

            self.assertEqual([site.site_ref() for site in inventory], [
                "libraries/forge/b.lua:1",
                "packages/example/a.lua:1",
            ])
            self.assertEqual([site.detail for site in inventory], [
                f"duplicate_function: repeated {entry.body_hash}",
                f"duplicate_function: repeated {entry.body_hash}",
            ])

    def test_code_dedup_child_issue_defines_group_owned_allowlist_contract(self) -> None:
        spec = slicer.specs()["code-dedup"]
        entry = "safe_segment 355dd98be98f94eb14fba5095f24ee8c packages/a.lua packages/b.lua"
        inventory = [
            slicer.InventorySite("packages/a.lua", 10, "duplicate_function: safe_segment 355dd98be98f94eb14fba5095f24ee8c", entry),
            slicer.InventorySite("packages/b.lua", 20, "duplicate_function: safe_segment 355dd98be98f94eb14fba5095f24ee8c", entry),
        ]

        body = slicer.render_child_issue(spec, inventory, 2)

        self.assertIn("## Allowlist Contract", body)
        self.assertIn(
            "- `migration/code-dedup.allowlist` is a shrink-only debt ledger, not an alternate duplicate inventory.",
            body,
        )
        self.assertIn(
            "- The authoritative current inventory is derived from `check_repo_dedup.duplicate_groups`; "
            "an allowlist line is retained only while that exact duplicate group still exists.",
            body,
        )
        self.assertIn(
            "- A `code-dedup` allowlist line is owned as one group by its function name, body hash, and listed file set.",
            body,
        )
        self.assertIn(
            "- After a selected group is migrated so the exact duplicate group no longer exists, remove the whole matching allowlist line; "
            "do not preserve a reduced singleton entry such as `safe_segment` as a live allowlist exception.",
            body,
        )
        self.assertIn(
            "- This spec-only slice explicitly waives a migration-slicer recurrence-class fix; "
            "any broader slicer deduplication change must be tracked separately.",
            body,
        )

    def test_code_dedup_stale_singleton_allowlist_entry_is_not_live_inventory(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            first = root / "packages/example/a.lua"
            second = root / "packages/example/b.lua"
            first.parent.mkdir(parents=True)
            first.write_text(
                textwrap.dedent(
                    """\
                    local function safe_segment(value)
                      return tostring(value or ""):gsub("[^%w._/-]", "-")
                    end
                    """
                ),
                encoding="utf-8",
            )
            second.write_text("return {}\n", encoding="utf-8")
            migration = root / "migration"
            migration.mkdir()
            (migration / "code-dedup.allowlist").write_text(
                "safe_segment 355dd98be98f94eb14fba5095f24ee8c packages/example/a.lua packages/example/b.lua\n",
                encoding="utf-8",
            )

            source_map = slicer.code_dedup.sources(root, root / "packages", slicer.read_text, slicer.repo_rel)
            allowlist = slicer.code_dedup.load_allowlist(migration / "code-dedup.allowlist")
            messages = slicer.code_dedup.ratchet_messages(source_map, allowlist, base_allowlist=allowlist)

        self.assertEqual(len(messages), 1)
        self.assertIn("safe_segment", messages[0])
        self.assertIn("no longer matches a duplicate group", messages[0])
        self.assertIn("prune the stale entry", messages[0])

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
                    str(SLICER_PATH),
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
            # selected_count = min(slice_size=2, remaining_count); as a ratchet is
            # drained toward zero its remaining count can be 1 (< slice_size), so the
            # dry-run selects 1 site. Accept 0-2 instead of hard-coding 0 or 2.
            self.assertRegex(result.stdout, r"- selected_count: [0-2]\b")
            self.assertIn("## Acceptance Criteria", result.stdout)

    def test_current_repo_ratchets_print_json_schema(self) -> None:
        # Validate the slicer JSON schema against every registered ratchet by its
        # ACTUAL current status. This stays correct as ratchets are migrated to
        # zero: a ratchet's status flips slice_available -> inventory_empty once its
        # allowlist is drained (e.g. code-dedup), so the test must not hard-code
        # which ratchets still have inventory.
        for ratchet in slicer.specs():
            result = subprocess.run(
                [
                    "python3",
                    "-B",
                    str(SLICER_PATH),
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
            self.assertEqual(doc["schema_version"], "fkst.ratchet-slice.v1")
            self.assertEqual(doc["ratchet"], ratchet)
            self.assertIn(doc["status"], ("slice_available", "inventory_empty"))
            if doc["status"] == "slice_available":
                self.assertEqual(
                    doc["next_slice"]["dedup_key"],
                    f"{ratchet}/slice/{doc['next_slice']['dedup_key'].split('/')[-1]}",
                )
            else:
                self.assertEqual(doc["remaining_count"], 0)
                self.assertIsNone(doc["next_slice"])


if __name__ == "__main__":
    unittest.main()
