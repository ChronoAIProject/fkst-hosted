#!/usr/bin/env python3
"""Tests for the github-devloop forward-direct raise ratchet."""

from __future__ import annotations

import importlib.util
import sys
import tempfile
import unittest
from pathlib import Path


def load_module():
    path = Path(__file__).with_name("check_repo_forward_direct.py")
    spec = importlib.util.spec_from_file_location("check_repo_forward_direct", path)
    if spec is None or spec.loader is None:
        raise RuntimeError("could not load check_repo_forward_direct.py")
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


forward = load_module()


class ForwardDirectRatchetTest(unittest.TestCase):
    def test_allowlisted_forward_direct_site_passes(self) -> None:
        site = forward.ForwardDirectSite(
            "packages/github-devloop/departments/review_result/main.lua",
            "pipeline",
            "devloop_fixing",
        )
        self.assertEqual(forward.ratchet_messages({site}, {site}), [])

    def test_new_forward_direct_site_fails(self) -> None:
        site = forward.ForwardDirectSite(
            "packages/github-devloop/departments/review_result/main.lua",
            "pipeline",
            "devloop_fixing",
        )
        messages = forward.ratchet_messages({site}, set())
        self.assertEqual(len(messages), 1)
        self.assertIn("FORWARD-direct raise not in migration/forward-direct-raise.allowlist", messages[0])

    def test_merge_ready_queue_is_marker_gated(self) -> None:
        sites = forward.source_sites(
            "packages/github-devloop/departments/review_result/main.lua",
            'function pipeline(event)\n  core.log_raise("review_result", id, "devloop_merge_ready", payload)\nend\n',
        )
        messages = forward.ratchet_messages(sites, set())
        self.assertEqual(len(messages), 1)
        self.assertIn("devloop_merge_ready", messages[0])

    def test_stale_allowlist_site_fails(self) -> None:
        site = forward.ForwardDirectSite(
            "packages/github-devloop/departments/review_result/main.lua",
            "pipeline",
            "devloop_fixing",
        )
        messages = forward.ratchet_messages(set(), {site})
        self.assertEqual(len(messages), 1)
        self.assertIn("no longer exists", messages[0])

    def test_causal_comment_handoff_is_exempt(self) -> None:
        sites = forward.source_sites(
            "packages/github-devloop/departments/comment_handoff/main.lua",
            'function pipeline(event)\n  core.log_raise("comment_handoff", id, "devloop_ready", payload)\nend\n',
        )
        self.assertEqual(sites, set())

    def test_redrive_replayer_is_exempt(self) -> None:
        sites = forward.source_sites(
            "packages/github-devloop/core/replayer.lua",
            'return raise_effects(dept, id, nil, nil, {}, {{ queue = "devloop_ready", payload = payload }})\n',
        )
        self.assertEqual(sites, set())

    def test_source_scan_finds_helper_and_effect_sites(self) -> None:
        text = """
local function direct()
  core.log_raise("dept", id, "devloop_ready", payload)
end
function M.helper()
  return raise_effects(dept, id, nil, nil, {}, {
    { queue = "devloop_reviewing", payload = payload },
  })
end
"""
        sites = forward.source_sites("packages/github-devloop/core/example.lua", text)
        self.assertEqual(
            {site.allowlist_line() for site in sites},
            {
                "packages/github-devloop/core/example.lua|direct|devloop_ready",
                "packages/github-devloop/core/example.lua|M.helper|devloop_reviewing",
            },
        )

    def test_inline_saga_act_is_pipeline_site(self) -> None:
        text = """
return saga.department(spec, { done = function() return false end, act = function(event)
  core.log_raise("dept", id, "devloop_reconcile", payload)
end })
"""
        sites = forward.source_sites("packages/github-devloop/departments/loop/main.lua", text)
        self.assertEqual(
            {site.allowlist_line() for site in sites},
            {"packages/github-devloop/departments/loop/main.lua|pipeline|devloop_reconcile"},
        )

    def test_source_scan_classifies_known_dynamic_recovery_queue(self) -> None:
        text = """
local function maybe_redrive_not_mergeable_pr()
  core.log_raise("observe_pr", id, recovery.queue, payload)
end
"""
        sites = forward.source_sites("packages/github-devloop/departments/observe_pr/main.lua", text)
        self.assertEqual(
            {site.allowlist_line() for site in sites},
            {"packages/github-devloop/departments/observe_pr/main.lua|maybe_redrive_not_mergeable_pr|devloop_fixing"},
        )

    def test_repository_messages_loads_allowlist(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            dept = root / "packages" / "github-devloop" / "departments" / "x"
            dept.mkdir(parents=True)
            source = dept / "main.lua"
            source.write_text(
                'function pipeline(event)\n  core.log_raise("x", id, "devloop_merge_ready", payload)\nend\n',
                encoding="utf-8",
            )
            migration = root / "migration"
            migration.mkdir()
            (migration / "forward-direct-raise.allowlist").write_text(
                "packages/github-devloop/departments/x/main.lua|pipeline|devloop_merge_ready\n",
                encoding="utf-8",
            )
            self.assertEqual(forward.repository_messages(root), [])


if __name__ == "__main__":
    unittest.main()
