#!/usr/bin/env python3
"""Unit tests for the G-SPAN repository guard."""

from __future__ import annotations

import importlib.util
import sys
import textwrap
import unittest
from pathlib import Path


def load_module():
    path = Path(__file__).with_name("check_repo_span.py")
    spec = importlib.util.spec_from_file_location("check_repo_span", path)
    if spec is None or spec.loader is None:
        raise RuntimeError("could not load check_repo_span.py")
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


span = load_module()


class SpanContractRatchetTest(unittest.TestCase):
    def test_completion_fact_named_started_with_head_sha_fails(self) -> None:
        sources = {
            "packages/github-devloop/core/strings.lua": textwrap.dedent(
                """\
                local strings = {
                  en = {
                    implementation_started = "github-devloop implementation started",
                  },
                }
                """
            ),
            "packages/github-devloop/core/requests/lifecycle.lua": textwrap.dedent(
                """\
                function M.build_implementing_comment_request(repo, issue_number, ready, worktree, branch, head_sha)
                  return {
                    body = M.comment_string("implementation_started")
                      .. "\\nHead: " .. tostring(head_sha)
                  }
                end
                """
            ),
        }

        messages = span.completion_fact_name_messages(sources)

        self.assertEqual(len(messages), 1)
        self.assertIn("completion/output comment uses start wording", messages[0])
        self.assertIn("implementation_started", messages[0])

    def test_completion_fact_output_name_passes(self) -> None:
        sources = {
            "packages/github-devloop/core/strings.lua": textwrap.dedent(
                """\
                local strings = {
                  en = {
                    implementation_output_published = "github-devloop implementation output published",
                  },
                }
                """
            ),
            "packages/github-devloop/core/requests/lifecycle.lua": textwrap.dedent(
                """\
                function M.build_implementing_comment_request(repo, issue_number, ready, worktree, branch, head_sha)
                  return {
                    body = M.comment_string("implementation_output_published")
                      .. "\\nHead: " .. tostring(head_sha)
                  }
                end
                """
            ),
        }

        self.assertEqual(span.completion_fact_name_messages(sources), [])

    def test_completion_fact_named_started_literal_fails(self) -> None:
        sources = {
            "packages/github-devloop/core/requests/lifecycle.lua": textwrap.dedent(
                """\
                function M.build_implementing_comment_request(repo, issue_number, ready, worktree, branch, head_sha)
                  return {
                    body = "github-devloop implementation started"
                      .. "\\nHead: " .. tostring(head_sha)
                  }
                end
                """
            ),
        }

        messages = span.completion_fact_name_messages(sources)

        self.assertEqual(len(messages), 1)
        self.assertIn("completion/output comment uses start wording", messages[0])
        self.assertIn("literal", messages[0])

    def test_worker_spawn_without_declared_span_contract_fails(self) -> None:
        transition_sources = {
            "packages/github-devloop/core/restart/transitions/implementing.lua": textwrap.dedent(
                """\
                return function(M, h)
                  return {
                    from_state = "implementing",
                    driving_queue = "devloop_ready",
                    responsibility_signature = responsibility_signature({
                      state_kind = "worker",
                    }),
                  }
                end
                """
            )
        }
        department_sources = {
            "packages/github-devloop/departments/implement/main.lua": "local result = spawn_codex_sync({ prompt = prompt })\n"
        }

        messages = span.spawn_start_messages(transition_sources, department_sources)

        self.assertEqual(len(messages), 1)
        self.assertIn("worker row with spawn_codex_sync must declare span_contract", messages[0])

    def test_spawn_before_declared_start_predecessor_fails(self) -> None:
        transition_sources = {
            "packages/github-devloop/core/restart/transitions/implementing.lua": textwrap.dedent(
                """\
                return function(M, h)
                  local span_contract = h.span_contract
                  return {
                    from_state = "implementing",
                    driving_queue = "devloop_ready",
                    responsibility_signature = responsibility_signature({
                      state_kind = "worker",
                    }),
                    span_contract = span_contract({
                      department = "implement",
                      durable_start_marker = "implement-attempt:v1",
                      spawn_predecessor = "raise_implementing_state",
                    }),
                  }
                end
                """
            )
        }
        department_sources = {
            "packages/github-devloop/departments/implement/main.lua": textwrap.dedent(
                """\
                local function raise_implementing_state(repo, issue_number, ready)
                  local marker = core.implement_attempt_marker(ready.proposal_id, ready.dedup_key, 1, now())
                  raise("github-proxy.github_issue_comment_request", { body = marker })
                end

                local result = spawn_codex_sync({ prompt = prompt })
                raise_implementing_state(repo, issue_number, ready, worktree, branch, base_branch, base_sha, attempt, started_at)
                """
            )
        }

        messages = span.spawn_start_messages(transition_sources, department_sources)

        self.assertEqual(len(messages), 1)
        self.assertIn("spawn_codex_sync must be preceded by span start predecessor", messages[0])
        self.assertIn("raise_implementing_state", messages[0])

    def test_predecessor_function_definition_without_call_does_not_satisfy_spawn(self) -> None:
        transition_sources = {
            "packages/github-devloop/core/restart/transitions/implementing.lua": textwrap.dedent(
                """\
                return function(M, h)
                  local span_contract = h.span_contract
                  return {
                    from_state = "implementing",
                    driving_queue = "devloop_ready",
                    responsibility_signature = responsibility_signature({
                      state_kind = "worker",
                    }),
                    span_contract = span_contract({
                      department = "implement",
                      durable_start_marker = "implement-attempt:v1",
                      spawn_predecessor = "raise_implementing_state",
                    }),
                  }
                end
                """
            )
        }
        department_sources = {
            "packages/github-devloop/departments/implement/main.lua": textwrap.dedent(
                """\
                local function raise_implementing_state()
                  local marker = core.implement_attempt_marker(ready.proposal_id, ready.dedup_key, 1, now())
                end

                local result = spawn_codex_sync({ prompt = prompt })
                """
            )
        }

        messages = span.spawn_start_messages(transition_sources, department_sources)

        self.assertEqual(len(messages), 1)
        self.assertIn("spawn_codex_sync must be preceded by span start predecessor", messages[0])

    def test_declared_start_predecessor_before_spawn_passes(self) -> None:
        transition_sources = {
            "packages/github-devloop/core/restart/transitions/implementing.lua": textwrap.dedent(
                """\
                return function(M, h)
                  local span_contract = h.span_contract
                  return {
                    from_state = "implementing",
                    driving_queue = "devloop_ready",
                    responsibility_signature = responsibility_signature({
                      state_kind = "worker",
                    }),
                    span_contract = span_contract({
                      department = "implement",
                      durable_start_marker = "implement-attempt:v1",
                      spawn_predecessor = "raise_implementing_state",
                    }),
                  }
                end
                """
            )
        }
        department_sources = {
            "packages/github-devloop/departments/implement/main.lua": textwrap.dedent(
                """\
                local function raise_implementing_state(repo, issue_number, ready)
                  local marker = core.implement_attempt_marker(ready.proposal_id, ready.dedup_key, 1, now())
                  raise("github-proxy.github_issue_comment_request", { body = marker })
                end

                raise_implementing_state(repo, issue_number, ready, worktree, branch, base_branch, base_sha, attempt, started_at)
                local result = spawn_codex_sync({ prompt = prompt })
                """
            )
        }

        self.assertEqual(span.spawn_start_messages(transition_sources, department_sources), [])

    def test_declared_start_predecessor_must_emit_durable_start_marker(self) -> None:
        transition_sources = {
            "packages/github-devloop/core/restart/transitions/implementing.lua": textwrap.dedent(
                """\
                return function(M, h)
                  local span_contract = h.span_contract
                  return {
                    from_state = "implementing",
                    driving_queue = "devloop_ready",
                    responsibility_signature = responsibility_signature({
                      state_kind = "worker",
                    }),
                    span_contract = span_contract({
                      department = "implement",
                      durable_start_marker = "implement-attempt:v1",
                      spawn_predecessor = "raise_implementing_state",
                    }),
                  }
                end
                """
            )
        }
        department_sources = {
            "packages/github-devloop/departments/implement/main.lua": textwrap.dedent(
                """\
                local function raise_implementing_state(repo, issue_number, ready)
                  local marker = "<!-- fkst:github-devloop:wrong-marker:v1 -->"
                  raise("github-proxy.github_issue_comment_request", { body = marker })
                end

                raise_implementing_state(repo, issue_number, ready)
                local result = spawn_codex_sync({ prompt = prompt })
                """
            )
        }

        messages = span.spawn_start_messages(transition_sources, department_sources)

        self.assertEqual(len(messages), 1)
        self.assertIn("span start predecessor", messages[0])
        self.assertIn("does not bind durable start marker", messages[0])
        self.assertIn("implement-attempt:v1", messages[0])

    def test_declared_state_start_predecessor_can_validate_declared_state_marker(self) -> None:
        transition_sources = {
            "packages/github-devloop/core/restart/transitions/fixing.lua": textwrap.dedent(
                """\
                return function(M, h)
                  local span_contract = h.span_contract
                  return {
                    from_state = "fixing",
                    driving_queue = "devloop_fixing",
                    responsibility_signature = responsibility_signature({
                      state_kind = "worker",
                    }),
                    span_contract = span_contract({
                      department = "fix",
                      durable_start_marker = "state:v1 fixing",
                      spawn_predecessor = "precheck_fix_write_gate",
                      spawn_function = "run_fix_attempt",
                    }),
                  }
                end
                """
            )
        }
        department_sources = {
            "packages/github-devloop/departments/fix/main.lua": textwrap.dedent(
                """\
                local function validate_fix_write_gate_snapshot(pr, fix)
                  local rechecked_state = core.current_entity_state(pr.comments, fix.proposal_id)
                  if rechecked_state.state ~= "fixing" then
                    return nil
                  end
                  return pr
                end

                local function precheck_fix_write_gate(repo, fix, branch)
                  return validate_fix_write_gate_snapshot(pr, fix) ~= nil
                end

                local function run_fix_attempt(plan)
                  local result = spawn_codex_sync({ prompt = prompt })
                  return result
                end

                precheck_fix_write_gate(repo, fix, branch)
                local outcome = run_fix_attempt(attempt_plan)
                """
            )
        }

        self.assertEqual(span.spawn_start_messages(transition_sources, department_sources), [])

    def test_declared_spawn_function_checks_predecessor_before_function_call(self) -> None:
        transition_sources = {
            "packages/github-devloop/core/restart/transitions/fixing.lua": textwrap.dedent(
                """\
                return function(M, h)
                  local span_contract = h.span_contract
                  return {
                    from_state = "fixing",
                    driving_queue = "devloop_fixing",
                    responsibility_signature = responsibility_signature({
                      state_kind = "worker",
                    }),
                    span_contract = span_contract({
                      department = "fix",
                      durable_start_marker = "state:v1 fixing",
                      spawn_predecessor = "precheck_fix_write_gate",
                      spawn_function = "run_fix_attempt",
                    }),
                  }
                end
                """
            )
        }
        department_sources = {
            "packages/github-devloop/departments/fix/main.lua": textwrap.dedent(
                """\
                local function validate_fix_write_gate_snapshot(pr, fix)
                  local rechecked_state = core.current_entity_state(pr.comments, fix.proposal_id)
                  if rechecked_state.state ~= "fixing" then
                    return nil
                  end
                  return pr
                end

                local function precheck_fix_write_gate(repo, fix, branch)
                  return validate_fix_write_gate_snapshot(pr, fix) ~= nil
                end

                local function run_fix_attempt(plan)
                  local result = spawn_codex_sync({ prompt = prompt })
                  return result
                end

                precheck_fix_write_gate(repo, fix, branch)
                local outcome = run_fix_attempt(attempt_plan)
                """
            )
        }

        self.assertEqual(span.spawn_start_messages(transition_sources, department_sources), [])


if __name__ == "__main__":
    unittest.main()
