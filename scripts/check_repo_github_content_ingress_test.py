#!/usr/bin/env python3
"""Tests for GitHub authored-content ingress ratchet."""

from __future__ import annotations

import tempfile
import unittest
from pathlib import Path

from check_repo_test import load_check_repo


check_repo = load_check_repo()


class GithubContentIngressGuardTest(unittest.TestCase):
    def run_guard(self, files: dict[str, str]) -> list[str]:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            for relpath, source in files.items():
                path = root / relpath
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text(source, encoding="utf-8")
            violations: list[str] = []
            check_repo.check_github_content_ingress(root, violations)
            return violations

    def base_files(self) -> dict[str, str]:
        return {
            "libraries/forge/github/exec.lua": """
local content_filter = require("forge.github.content_filter")
local stdout_policy = require("forge.github.stdout_policy")
local function f(result, policy)
  if stdout_policy.is_content_json(policy) then
    return content_filter.apply_gh_content_filter(result, nil, policy, {}, stdout_policy)
  end
end
""",
            "libraries/devloop/gh_exec.lua": """
local content_filter = require("forge.github.content_filter")
local stdout_policy = require("forge.github.stdout_policy")
local function f(result, policy)
  if stdout_policy.is_content_json(policy) then
    return content_filter.apply_gh_content_filter(result, nil, policy, {}, stdout_policy)
  end
end
""",
        }

    def test_allows_two_mediated_wrappers_and_declared_policy(self) -> None:
        files = self.base_files()
        files["libraries/forge/github/issue.lua"] = """
function M.install(handle)
  return handle._exec({"gh", "issue", "view", "1"}, 30, "gh issue view", stdout_policy.content_json("issue_view"))
end
"""
        self.assertEqual(self.run_guard(files), [])

    def test_requires_both_wrappers_to_filter(self) -> None:
        files = self.base_files()
        files["libraries/devloop/gh_exec.lua"] = "local stdout_policy = require('forge.github.stdout_policy')\n"
        violations = self.run_guard(files)
        self.assertEqual(len(violations), 1)
        self.assertTrue(all("G-GITHUB-CONTENT-INGRESS" in item for item in violations))
        self.assertTrue(all("libraries/devloop/gh_exec.lua" in item for item in violations))

    def test_rejects_missing_stdout_policy_on_authored_read(self) -> None:
        files = self.base_files()
        files["libraries/forge/github/issue.lua"] = """
function M.install(handle)
  return handle._exec({"gh", "issue", "view", "1"}, 30, "gh issue view")
end
"""
        violations = self.run_guard(files)
        self.assertEqual(len(violations), 1)
        self.assertIn("must declare a stdout_policy", violations[0])

    def test_rejects_third_raw_gh_exec_argv_egress(self) -> None:
        files = self.base_files()
        files["packages/github-devloop/core/raw.lua"] = """
function M.bad()
  return exec_argv({ argv = { "gh", "api", "repos/o/r/issues/1" }, timeout = 30 })
end
"""
        violations = self.run_guard(files)
        self.assertEqual(len(violations), 1)
        self.assertIn("raw gh exec_argv egress", violations[0])

    def test_rejects_policyless_production_github_handle_construction(self) -> None:
        files = self.base_files()
        files["packages/github-devloop/core/raw.lua"] = """
function M.bad()
  return require("forge.github").new(exec_argv).issue_view("owner/repo", 42, "number,title,body", 30)
end
"""
        violations = self.run_guard(files)
        self.assertEqual(len(violations), 1)
        self.assertIn("production forge.github construction", violations[0])

    def test_allows_devloop_policy_factory_construction(self) -> None:
        files = self.base_files()
        files["libraries/devloop/github_factory.lua"] = """
local github_adapter = require("forge.github")
function M.new(exec)
  return github_adapter.new(exec, require("devloop.github_author_policy").github_options(exec))
end
"""
        files["packages/github-devloop/core/good.lua"] = """
function M.good()
  return require("devloop.github_factory").production_handle().issue_view("owner/repo", 42, "number,title,body", 30)
end
"""
        self.assertEqual(self.run_guard(files), [])

    def test_allows_forge_ports_explicit_policy_threading(self) -> None:
        files = self.base_files()
        files["libraries/forge/ports.lua"] = """
function M.production_handles(opts)
  return {
    github = require("forge.github").new(exec_argv, {
      trusted_author_policy = opts.trusted_author_policy,
    }),
  }
end
"""
        self.assertEqual(self.run_guard(files), [])

    def test_rejects_forge_ports_policyless_github_construction(self) -> None:
        files = self.base_files()
        files["libraries/forge/ports.lua"] = """
function M.production_handles(opts)
  return {
    github = require("forge.github").new(exec_argv, opts),
  }
end
"""
        violations = self.run_guard(files)
        self.assertEqual(len(violations), 1)
        self.assertIn("production forge.github construction", violations[0])

    def test_rejects_forge_merge_production_fallback(self) -> None:
        files = self.base_files()
        files["libraries/forge/merge/verified_merge.lua"] = """
local github_adapter = require("forge.github")
function S.install(M, shared, opts)
  local github = (opts and opts.github_handle) or github_adapter.production_handle
  return github("forge.merge").gh_pr_view_merge("owner/repo", 7, 30)
end
"""
        violations = self.run_guard(files)
        self.assertEqual(len(violations), 1)
        self.assertIn("production forge.github construction", violations[0])

    def test_rejects_authored_api_path_with_metadata_policy(self) -> None:
        files = self.base_files()
        files["libraries/forge/github/entities.lua"] = """
function M.install(handle)
  return handle._exec({"gh", "api", "repos/owner/repo/issues?state=open"}, 30, "gh api", stdout_policy.trusted_metadata_json())
end
"""
        violations = self.run_guard(files)
        self.assertEqual(len(violations), 1)
        self.assertIn("authored GitHub API read must declare stdout_policy.content_json", violations[0])

    def test_rejects_authored_list_helpers_with_metadata_policy(self) -> None:
        helper_cases = {
            "issue_list_open_assigned": ('issue_list_open_assigned_argv(repo, assignee)', "issue_list"),
            "pr_list_recent_merged": ('pr_list_recent_merged_argv(repo, limit)', "pr_list"),
            "pr_list_head": ('pr_list_head_argv(repo, branch, base)', "pr_list"),
            "pr_list_merge_queue": ('pr_list_merge_queue_argv(repo, base)', "pr_list"),
        }
        for helper, (argv_call, shape) in helper_cases.items():
            with self.subTest(helper=helper):
                files = self.base_files()
                files["libraries/forge/github/entities.lua"] = f"""
function M.install(handle)
  function handle.{helper}(repo)
    return handle._exec({argv_call}, 30, "gh list", stdout_policy.trusted_metadata_json())
  end
end
"""
                violations = self.run_guard(files)
                self.assertEqual(len(violations), 1)
                self.assertIn(f"{helper} must declare stdout_policy.content_json(\"{shape}\")", violations[0])


if __name__ == "__main__":
    unittest.main()
