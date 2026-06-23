#!/usr/bin/env python3
"""Tests for the github-devloop saga split migration ratchet."""

from __future__ import annotations

import importlib.util
import json
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock


def load_module():
    path = Path(__file__).with_name("check_repo_saga_split.py")
    spec = importlib.util.spec_from_file_location("check_repo_saga_split", path)
    if spec is None or spec.loader is None:
        raise RuntimeError("could not load check_repo_saga_split.py")
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


saga_split = load_module()


class SagaSplitRatchetTest(unittest.TestCase):
    def write_manifest(self, root: Path, rows: list[dict[str, str]]) -> None:
        migration = root / "migration"
        migration.mkdir(exist_ok=True)
        (migration / "github-devloop-saga-split.inventory").write_text(
            "\n".join(json.dumps(row, separators=(",", ":")) for row in rows) + "\n",
            encoding="utf-8",
        )

    def write_allowlist(self, root: Path, lines: list[str]) -> None:
        migration = root / "migration"
        migration.mkdir(exist_ok=True)
        (migration / "github-devloop-saga-split-authority.allowlist").write_text(
            "\n".join(lines) + ("\n" if lines else ""),
            encoding="utf-8",
        )

    def make_repo(self) -> tuple[tempfile.TemporaryDirectory[str], Path]:
        tmp = tempfile.TemporaryDirectory()
        root = Path(tmp.name)
        (root / "packages" / "github-devloop" / "departments" / "observe_issue").mkdir(parents=True)
        (root / "packages" / "github-devloop" / "departments" / "review_result").mkdir(parents=True)
        (root / "packages" / "github-devloop" / "core").mkdir(parents=True)
        (root / "libraries" / "devloop" / "restart" / "issue").mkdir(parents=True)
        (root / "packages" / "github-devloop" / "departments" / "observe_issue" / "main.lua").write_text(
            "local core = require('core')\nreturn {}\n",
            encoding="utf-8",
        )
        (root / "packages" / "github-devloop" / "departments" / "review_result" / "main.lua").write_text(
            "local core = require('core')\nreturn {}\n",
            encoding="utf-8",
        )
        (root / "packages" / "github-devloop" / "core" / "entity.lua").write_text(
            "local M = {}\nreturn M\n",
            encoding="utf-8",
        )
        (root / "libraries" / "devloop" / "restart" / "issue" / "pr_partition_contract.lua").write_text(
            """local PR_PHASE_STATES = {
  "pr-open",
  "reviewing",
  "fixing",
  "review-meta",
  "merge-ready",
  "merging",
}
return {}
""",
            encoding="utf-8",
        )
        return tmp, root

    def base_rows(self) -> list[dict[str, str]]:
        return [
            {
                "path": "packages/github-devloop/departments/observe_issue/main.lua",
                "owner": "issue",
                "reason": "issue owner",
            },
            {
                "path": "packages/github-devloop/departments/review_result/main.lua",
                "owner": "pr",
                "reason": "pr owner",
            },
            {
                "path": "packages/github-devloop/core/entity.lua",
                "owner": "cross-cutting",
                "reason": "linked entity migration debt",
            },
            {
                "path": "libraries/devloop/restart/issue/pr_partition_contract.lua",
                "owner": "shared",
                "reason": "inert partition contract",
            },
        ]

    def repository_messages(self, root: Path) -> list[str]:
        with mock.patch.object(saga_split, "allowlist_at_dev_base", return_value=("present", saga_split.load_allowlist(root / saga_split.ALLOWLIST))):
            return saga_split.repository_messages(root)

    def test_exhaustive_manifest_passes(self) -> None:
        tmp, root = self.make_repo()
        with tmp:
            self.write_manifest(root, self.base_rows())
            self.write_allowlist(root, [])
            self.assertEqual(self.repository_messages(root), [])

    def test_nested_std_devloop_restart_modules_are_inventory_paths(self) -> None:
        tmp, root = self.make_repo()
        with tmp:
            restart_nested = root / "libraries" / "devloop" / "restart" / "issue" / "transitions"
            restart_nested.mkdir(parents=True)
            (restart_nested / "index.lua").write_text("return {}\n", encoding="utf-8")
            rows = self.base_rows() + [
                {
                    "path": "libraries/devloop/restart/issue/transitions/index.lua",
                    "owner": "shared",
                    "reason": "nested shared restart helper",
                }
            ]
            self.write_manifest(root, rows)
            self.write_allowlist(root, [])
            self.assertEqual(self.repository_messages(root), [])

    def test_pr_phase_states_are_derived_from_lua_contract(self) -> None:
        tmp, root = self.make_repo()
        with tmp:
            self.assertEqual(
                saga_split.load_pr_phase_states(root),
                {"pr-open", "reviewing", "fixing", "review-meta", "merge-ready", "merging"},
            )

    def test_pr_phase_parser_fails_loudly_when_contract_block_is_missing(self) -> None:
        tmp, root = self.make_repo()
        with tmp:
            (root / saga_split.CONTRACT).write_text("return {}\n", encoding="utf-8")
            with self.assertRaisesRegex(ValueError, "contract-malformed"):
                saga_split.load_pr_phase_states(root)

    def test_pr_phase_parser_fails_loudly_when_contract_block_is_empty(self) -> None:
        tmp, root = self.make_repo()
        with tmp:
            (root / saga_split.CONTRACT).write_text("local PR_PHASE_STATES = {}\nreturn {}\n", encoding="utf-8")
            with self.assertRaisesRegex(ValueError, "contract-malformed"):
                saga_split.load_pr_phase_states(root)

    def test_missing_stale_and_duplicate_manifest_entries_fail(self) -> None:
        tmp, root = self.make_repo()
        with tmp:
            rows = [
                self.base_rows()[0],
                self.base_rows()[0],
                {
                    "path": "packages/github-devloop/core/stale.lua",
                    "owner": "shared",
                    "reason": "stale",
                },
            ]
            self.write_manifest(root, rows)
            self.write_allowlist(root, [])
            messages = self.repository_messages(root)
            self.assertTrue(any("manifest-missing-entry" in message for message in messages))
            self.assertTrue(any("manifest-stale-entry" in message for message in messages))
            self.assertTrue(any("manifest-duplicate-entry" in message for message in messages))

    def test_new_issue_owned_pr_state_marker_fails_but_pr_owned_passes(self) -> None:
        tmp, root = self.make_repo()
        with tmp:
            self.write_manifest(root, self.base_rows())
            self.write_allowlist(root, [])
            (root / "packages/github-devloop/departments/observe_issue/main.lua").write_text(
                'local function x()\n  return core.state_marker(id, "merge-ready", version)\nend\n',
                encoding="utf-8",
            )
            issue_messages = self.repository_messages(root)
            self.assertTrue(any("leak-new" in message and "merge-ready" in message for message in issue_messages))

            (root / "packages/github-devloop/departments/observe_issue/main.lua").write_text("return {}\n", encoding="utf-8")
            (root / "packages/github-devloop/departments/review_result/main.lua").write_text(
                'local function x()\n  return core.state_marker(id, "merge-ready", version)\nend\n',
                encoding="utf-8",
            )
            self.assertEqual(self.repository_messages(root), [])

    def test_multiline_issue_owned_pr_phase_state_marker_fails(self) -> None:
        tmp, root = self.make_repo()
        with tmp:
            self.write_manifest(root, self.base_rows())
            self.write_allowlist(root, [])
            target = root / "packages/github-devloop/departments/observe_issue/main.lua"
            target.write_text(
                """local function x()
  return core.state_marker(
    id,
    "merge-ready",
    version
  )
end
""",
                encoding="utf-8",
            )
            messages = self.repository_messages(root)
            self.assertTrue(any("leak-new" in message and "merge-ready" in message for message in messages), messages)

    def test_issue_to_pr_boundary_seed_pr_open_marker_is_allowed(self) -> None:
        tmp, root = self.make_repo()
        with tmp:
            rows = self.base_rows() + [
                {
                    "path": "packages/github-devloop/core/pr_delegation.lua",
                    "owner": "issue",
                    "reason": "issue-to-pr boundary seed",
                }
            ]
            self.write_manifest(root, rows)
            self.write_allowlist(root, [])
            (root / "packages/github-devloop" / "core" / "pr_delegation.lua").write_text(
                """local function build_pr_open_comment_request(issue_proposal_id, impl_version)
  local body = M.pr_origin_marker(issue_proposal_id, 42, "branch", impl_version, "base")
    .. "\\n" .. M.state_marker(issue_proposal_id, "pr-open", impl_version)
  return body
end
""",
                encoding="utf-8",
            )

            self.assertEqual(self.repository_messages(root), [])

    def test_shared_label_builder_pr_phase_vocabulary_is_not_an_authority_leak(self) -> None:
        tmp, root = self.make_repo()
        with tmp:
            rows = []
            for row in self.base_rows():
                copied = dict(row)
                if copied["path"] == "packages/github-devloop/core/entity.lua":
                    copied["owner"] = "shared"
                rows.append(copied)
            self.write_manifest(root, rows)
            self.write_allowlist(root, [])
            (root / "packages/github-devloop/core/entity.lua").write_text(
                """local function x()
  return M.build_state_label_request(repo, issue_number, "merge-ready", dedup, source_ref)
end
""",
                encoding="utf-8",
            )
            self.assertEqual(self.repository_messages(root), [])

    def test_linked_state_promotion_leak_detected(self) -> None:
        tmp, root = self.make_repo()
        with tmp:
            self.write_manifest(root, self.base_rows())
            self.write_allowlist(root, [])
            (root / "packages/github-devloop/core/entity.lua").write_text(
                "function M.issue_authoritative_linked_state(issue_state, linked_state)\n  return linked_state or issue_state\nend\n",
                encoding="utf-8",
            )
            messages = self.repository_messages(root)
            self.assertTrue(any("linked-state-promotion" in message for message in messages))

    def test_stale_and_growing_allowlist_fail(self) -> None:
        tmp, root = self.make_repo()
        with tmp:
            self.write_manifest(root, self.base_rows())
            site = saga_split.LeakSite(
                "packages/github-devloop/core/entity.lua",
                "linked-state-promotion",
                "linked-pr-comments",
                1,
            )
            self.write_allowlist(root, [site.allowlist_line("current debt")])
            messages = self.repository_messages(root)
            self.assertTrue(any("allowlist-stale" in message for message in messages))

            (root / "packages/github-devloop/core/entity.lua").write_text(
                "function M.issue_authoritative_linked_state(issue_state, linked_state)\n  return linked_state or issue_state\nend\n",
                encoding="utf-8",
            )
            with mock.patch.object(saga_split, "allowlist_at_dev_base", return_value=("present", set())):
                growth = saga_split.repository_messages(root)
            self.assertTrue(any("allowlist-growth" in message for message in growth))


if __name__ == "__main__":
    unittest.main()
