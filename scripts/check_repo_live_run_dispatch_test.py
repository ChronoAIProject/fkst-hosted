#!/usr/bin/env python3
"""Tests for the live-run dispatch structural ratchet."""

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
live_run_dispatch = load_module("check_repo_live_run_dispatch", scripts_dir / "check_repo_live_run_dispatch.py")


def write_allowlist(root: Path, text: str) -> None:
    path = root / live_run_dispatch.ALLOWLIST
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(text, encoding="utf-8")


class LiveRunDispatchRatchetTest(unittest.TestCase):
    def test_accepts_path_line_allowlist_entries(self) -> None:
        entries = live_run_dispatch.parse_allowlist_lines([
            "# comment",
            "packages/example/departments/decide/main.lua:42",
            "libraries/workflow/codex.lua:7 # trailing",
            "",
        ])

        self.assertEqual(entries, {
            "packages/example/departments/decide/main.lua:42",
            "libraries/workflow/codex.lua:7",
        })

    def test_rejects_invalid_allowlist_entries(self) -> None:
        with self.assertRaises(ValueError):
            live_run_dispatch.parse_allowlist_lines(["packages/example/main.lua:not-a-line"])

    def test_allowlist_growth_fails_when_base_is_known(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            write_allowlist(root, "packages/example/departments/decide/main.lua:42\n")
            with mock.patch.object(live_run_dispatch, "allowlist_at_dev_base", return_value=("present", set())):
                messages = live_run_dispatch.repository_messages(root, enforce_base=True)

        self.assertEqual(messages, [
            "packages/example/departments/decide/main.lua:42 grows migration/live-run-dispatch.allowlist relative to dev; migrate the dispatch to workflow.codex.dispatch instead"
        ])

    def test_scans_raw_identity_spawn_and_allows_canonical_wrapper_only(self) -> None:
        sources = {
            "libraries/workflow/codex.lua": """
function M.dispatch(identity, opts)
  opts.role = identity.role
  opts.proposal_id = identity.proposal_id
  opts.dedup_key = identity.dedup_key
  return spawn_codex(opts)
end
""",
            "packages/example/departments/decide/main.lua": """
local function run(proposal)
  local opts = { prompt = "hello" }
  opts.role = "consensus"
  opts.proposal_id = proposal.proposal_id
  opts.dedup_key = proposal.dedup_key
  return spawn_codex(opts)
end
""",
        }

        sites = live_run_dispatch.current_violations(sources)
        self.assertEqual([site.allowlist_key() for site in sites], ["packages/example/departments/decide/main.lua:7"])
        self.assertEqual(
            live_run_dispatch.ratchet_messages(sites, allowlist=set(), base_allowlist=None),
            [
                "packages/example/departments/decide/main.lua:7 raw identity-carrying spawn_codex is forbidden outside workflow.codex.dispatch"
            ],
        )

    def test_repository_scan_excludes_fkst_runtime_and_finds_worktree_sources(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            worktree = root / "packages" / "example" / "departments" / "decide"
            installed = root / ".fkst" / "local-packages" / "example" / "departments" / "decide"
            worktree.mkdir(parents=True)
            installed.mkdir(parents=True)
            (worktree / "main.lua").write_text(
                "local workflow_codex = require('workflow.codex')\n"
                "local identity = require('contract.convergence_identity').from_parts('consensus', 'p', 'd', { angle_lane = 'worker' })\n"
                "return workflow_codex.dispatch(identity, { prompt = 'ok' })\n",
                encoding="utf-8",
            )
            (installed / "main.lua").write_text(
                "return spawn_codex({ role = 'consensus', proposal_id = 'p', dedup_key = 'd' })\n",
                encoding="utf-8",
            )

            messages = live_run_dispatch.repository_messages(root, enforce_base=False)

        self.assertEqual(messages, [])

    def test_check_repo_goes_red_for_new_raw_identity_dispatch(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            pkg = root / "packages" / "example" / "departments" / "decide"
            pkg.mkdir(parents=True)
            (pkg / "main.lua").write_text(
                "local function run(proposal)\n"
                "  return spawn_codex({\n"
                "    prompt = 'hello',\n"
                "    role = 'consensus',\n"
                "    proposal_id = proposal.proposal_id,\n"
                "    dedup_key = proposal.dedup_key,\n"
                "  })\n"
                "end\n",
                encoding="utf-8",
            )
            (root / "libraries").mkdir()
            (root / "migration").mkdir()
            (root / live_run_dispatch.ALLOWLIST).write_text("", encoding="utf-8")

            with mock.patch.object(live_run_dispatch, "allowlist_at_dev_base", return_value=("absent", None)):
                messages = live_run_dispatch.repository_messages(root, enforce_base=True)

        self.assertEqual(
            messages,
            [
                "packages/example/departments/decide/main.lua:2 raw identity-carrying spawn_codex is forbidden outside workflow.codex.dispatch"
            ],
        )

    def test_allowlist_fixture_long_string_does_not_preserve_leading_newline(self) -> None:
        entries = live_run_dispatch.parse_allowlist_lines("""
packages/example/departments/decide/main.lua:42
""".splitlines())

        self.assertEqual(entries, {"packages/example/departments/decide/main.lua:42"})

    def test_real_repo_allowlist_is_empty(self) -> None:
        root = Path(__file__).resolve().parents[1]
        with mock.patch.object(live_run_dispatch, "allowlist_at_dev_base", return_value=("absent", None)):
            messages = live_run_dispatch.repository_messages(root, enforce_base=True)

        self.assertEqual(messages, [])


if __name__ == "__main__":
    unittest.main()
