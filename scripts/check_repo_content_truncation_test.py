#!/usr/bin/env python3
"""Tests for the content-truncation shrink-only ratchet."""

from __future__ import annotations

import importlib.util
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path


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
content = check_repo.check_repo_content_truncation


def git(root: Path, *args: str) -> str:
    result = subprocess.run(
        ["git", *args],
        cwd=root,
        check=True,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )
    return result.stdout.strip()


def init_git_repo(root: Path) -> None:
    git(root, "init")
    git(root, "config", "user.email", "fkst-test@example.invalid")
    git(root, "config", "user.name", "fkst test")


def commit_paths(root: Path, paths: list[str], message: str) -> str:
    git(root, "add", *paths)
    git(root, "commit", "-m", message)
    return git(root, "rev-parse", "HEAD")


class ContentTruncationRatchetTest(unittest.TestCase):
    def test_detects_content_cap_raised_in_consensus_payload(self) -> None:
        source = """
local max_body_len = 12000
local function proposal_body(issue)
  return bounded_text(issue.body, max_body_len)
end
local function act(issue)
  local proposal = {
    schema = "consensus.proposal.v1",
    body = proposal_body(issue),
  }
  raise("consensus.proposal", proposal)
end
"""
        sites = content.source_sites("packages/example/core.lua", source)

        self.assertEqual(
            {site.key() for site in sites},
            {("packages/example/core.lua", "proposal_body", "max_body_len", "raise-payload")},
        )

    def test_detects_prompt_content_cap_before_codex_spawn(self) -> None:
        source = """
local max_context_len = 8000
local function build_review_prompt(issue)
  local context = truncate_utf8(issue.body, max_context_len)
  return "Context:\\n" .. context
end
local function run(issue)
  local prompt = build_review_prompt(issue)
  return spawn_codex_sync({ prompt = prompt })
end
"""
        sites = content.source_sites("packages/example/core.lua", source)

        self.assertEqual(
            {site.key() for site in sites},
            {("packages/example/core.lua", "build_review_prompt", "max_context_len", "codex-prompt")},
        )

    def test_ignores_key_marker_and_external_body_limits(self) -> None:
        source = """
local max_key_len = 200
local max_marker_value_len = 300
local max_body_len = 12000
local function safe_key(value)
  return tostring(value):sub(1, max_key_len)
end
local function marker(value)
  return truncate_utf8(value, max_marker_value_len)
end
local function issue_create(issue)
  local body = truncate_utf8(issue.body, max_body_len)
  return {
    schema = "github-proxy.issue-create.v1",
    body = body,
  }
end
"""
        self.assertEqual(content.source_sites("packages/example/core.lua", source), set())

    def test_allowlisted_site_passes_and_stale_entry_fails(self) -> None:
        site = content.ContentTruncationSite(
            "packages/example/core.lua",
            "proposal_body",
            "max_body_len",
            "raise-payload",
            3,
        )
        allow = {
            content.ContentTruncationSite.parse(
                "packages/example/core.lua|proposal_body|max_body_len|raise-payload|issue=#1117|why=legacy proposal body cap"
            )
        }

        self.assertEqual(content.ratchet_messages({site}, allow), [])
        messages = content.ratchet_messages(set(), allow)
        self.assertEqual(len(messages), 1)
        self.assertIn("prune the stale entry", messages[0])

    def test_allowlist_requires_issue_and_why(self) -> None:
        with self.assertRaises(ValueError):
            content.ContentTruncationSite.parse(
                "packages/example/core.lua|proposal_body|max_body_len|raise-payload|why=missing issue"
            )
        with self.assertRaises(ValueError):
            content.ContentTruncationSite.parse(
                "packages/example/core.lua|proposal_body|max_body_len|raise-payload|issue=#1117|why="
            )

    def test_allowlist_growth_relative_to_base_fails(self) -> None:
        entry = content.ContentTruncationSite.parse(
            "packages/example/core.lua|proposal_body|max_body_len|raise-payload|issue=#1117|why=legacy proposal body cap"
        )
        messages = content.ratchet_messages({entry}, {entry}, base_allowlist=set())

        self.assertEqual(len(messages), 1)
        self.assertIn("grows content-truncation allowlist relative to dev", messages[0])

    def test_dev_base_allowlist_resolves_from_origin_dev(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            init_git_repo(root)
            (root / "migration").mkdir()
            line = "packages/example/core.lua|proposal_body|max_body_len|raise-payload|issue=#1117|why=legacy proposal body cap\n"
            (root / content.ALLOWLIST).write_text(line, encoding="utf-8")
            base_commit = commit_paths(root, [content.ALLOWLIST], "base allowlist")
            git(root, "update-ref", "refs/remotes/origin/dev", base_commit)
            (root / content.ALLOWLIST).write_text("", encoding="utf-8")
            commit_paths(root, [content.ALLOWLIST], "head allowlist")

            status, allowlist = content.allowlist_at_dev_base(root)

        self.assertEqual(status, "present")
        self.assertEqual(
            {entry.key() for entry in allowlist or set()},
            {("packages/example/core.lua", "proposal_body", "max_body_len", "raise-payload")},
        )


if __name__ == "__main__":
    unittest.main()
