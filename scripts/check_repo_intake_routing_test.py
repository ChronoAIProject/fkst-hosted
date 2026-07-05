#!/usr/bin/env python3
"""Tests for the github-devloop-intake routing architecture ratchet."""

from __future__ import annotations

import importlib.util
import sys
import tempfile
import textwrap
import unittest
from pathlib import Path


def load_module():
    path = Path(__file__).with_name("check_repo_intake_routing.py")
    spec = importlib.util.spec_from_file_location("check_repo_intake_routing", path)
    if spec is None or spec.loader is None:
        raise RuntimeError("could not load check_repo_intake_routing.py")
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


intake_routing = load_module()


class IntakeRoutingRatchetTest(unittest.TestCase):
    def make_repo(self) -> tuple[tempfile.TemporaryDirectory[str], Path]:
        tmp = tempfile.TemporaryDirectory()
        root = Path(tmp.name)
        (root / "packages" / "github-devloop-intake" / "departments" / "admission").mkdir(parents=True)
        (root / "packages" / "github-devloop-intake-default" / "departments" / "intake_judge").mkdir(parents=True)
        (root / "packages" / "github-devloop-workflow" / "departments" / "workflow_select").mkdir(parents=True)
        (root / "packages" / "github-devloop-intake" / "core").mkdir(parents=True)
        (root / "scripts").mkdir()
        (root / "scripts" / "intake_policy_slots.json").write_text(
            textwrap.dedent(
                """\
                {
                  "schema": "fkst.package-topology-policy-slots.v1",
                  "policy_slots": [
                    {
                      "name": "intake-policy",
                      "consumer_queue": "github-devloop-intake.devloop_intake_candidate",
                      "implementations": [
                        {"package": "github-devloop-intake-default", "topology": "default"},
                        {"package": "github-devloop-workflow", "topology": "workflow"}
                      ]
                    }
                  ]
                }
                """
            ),
            encoding="utf-8",
        )
        self.write_intake_admission(root)
        self.write_default_consumer(root)
        (root / "packages" / "github-devloop-intake" / "core.lua").write_text(
            "local M = {}\nreturn M\n",
            encoding="utf-8",
        )
        return tmp, root

    def write_intake_admission(self, root: Path, produces: str | None = None, extra: str = "") -> None:
        produces_body = produces or '"devloop_intake_candidate", "github-proxy.github_issue_comment_request"'
        (root / "packages" / "github-devloop-intake" / "departments" / "admission" / "main.lua").write_text(
            textwrap.dedent(
                f"""\
                local spec = {{
                  consumes = {{ "github-proxy.github_entity_changed" }},
                  produces = {{ {produces_body} }},
                }}

                local function act(_event)
                  {extra}
                end

                return require("workflow.saga").department(spec, {{ act = act, done = function() return false end }})
                """
            ),
            encoding="utf-8",
        )

    def write_consumer(self, root: Path, package: str, department: str, queue: str = "github-devloop-intake.devloop_intake_candidate") -> None:
        path = root / "packages" / package / "departments" / department / "main.lua"
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(
            textwrap.dedent(
                f"""\
                local spec = {{
                  consumes = {{ "{queue}" }},
                  produces = {{ "github-devloop.devloop_execute_request" }},
                }}
                return require("workflow.saga").department(spec, {{ act = function() end, done = function() return false end }})
                """
            ),
            encoding="utf-8",
        )

    def write_default_consumer(self, root: Path, queue: str = "github-devloop-intake.devloop_intake_candidate") -> None:
        self.write_consumer(root, "github-devloop-intake-default", "intake_judge", queue)

    def write_workflow_consumer(self, root: Path, queue: str = "github-devloop-intake.devloop_intake_candidate") -> None:
        self.write_consumer(root, "github-devloop-workflow", "workflow_select", queue)

    def remove_default_consumer(self, root: Path) -> None:
        (root / "packages" / "github-devloop-intake-default" / "departments" / "intake_judge" / "main.lua").write_text(
            "return {}\n",
            encoding="utf-8",
        )

    def messages(self, root: Path) -> list[str]:
        return intake_routing.repository_messages(root)

    def assert_message_contains(self, messages: list[str], expected: str) -> None:
        self.assertTrue(
            any(expected in message for message in messages),
            f"expected message containing {expected!r}, got {messages!r}",
        )

    def test_only_intake_default_candidate_consumer_passes(self) -> None:
        tmp, root = self.make_repo()
        with tmp:
            self.assertEqual(self.messages(root), [])

    def test_only_workflow_candidate_consumer_passes(self) -> None:
        tmp, root = self.make_repo()
        with tmp:
            self.remove_default_consumer(root)
            self.write_workflow_consumer(root)
            self.assertEqual(self.messages(root), [])

    def test_both_policy_candidate_consumers_pass(self) -> None:
        tmp, root = self.make_repo()
        with tmp:
            self.write_workflow_consumer(root)
            self.assertEqual(self.messages(root), [])
            topology_messages = intake_routing.topology_exclusivity_messages(
                root,
                {"github-devloop-intake-default", "github-devloop-workflow"},
            )
            self.assert_message_contains(topology_messages, "policy slot 'intake-policy'")
            self.assert_message_contains(topology_messages, "github-devloop-intake-default")
            self.assert_message_contains(topology_messages, "github-devloop-workflow")

    def test_policy_slot_manifest_declares_legal_topology_exclusions(self) -> None:
        root = Path(__file__).resolve().parents[1]

        slot = intake_routing.intake_policy_slot(root)
        rows = {topology.name: set(topology.excluded_packages) for topology in intake_routing.legal_topologies(root)}

        self.assertEqual(slot.name, "intake-policy")
        self.assertEqual(slot.consumer_queue, "github-devloop-intake.devloop_intake_candidate")
        self.assertEqual(
            slot.packages,
            {"github-devloop-intake-default", "github-devloop-workflow"},
        )
        self.assertEqual(rows["default"], {"github-devloop-workflow"})
        self.assertEqual(rows["workflow"], {"github-devloop-intake-default"})

    def test_each_legal_topology_loads_exactly_one_policy(self) -> None:
        root = Path(__file__).resolve().parents[1]
        all_policy_packages = intake_routing.intake_policy_slot(root).packages

        for topology in intake_routing.legal_topologies(root):
            loaded = all_policy_packages - set(topology.excluded_packages)
            with self.subTest(topology=topology.name):
                self.assertEqual(intake_routing.topology_exclusivity_messages(root, loaded), [])

    def test_topology_without_policy_fails(self) -> None:
        root = Path(__file__).resolve().parents[1]

        messages = intake_routing.topology_exclusivity_messages(root, set())

        self.assert_message_contains(messages, "loaded none")

    def test_self_poll_raiser_fails(self) -> None:
        tmp, root = self.make_repo()
        with tmp:
            raiser = root / "packages" / "github-devloop-intake" / "raisers"
            raiser.mkdir()
            (raiser / "intake_poll.lua").write_text(
                'return { type = "cron", schedule = "*/5 * * * *", produces = "devloop_intake_candidate" }\n',
                encoding="utf-8",
            )

            messages = self.messages(root)

        self.assert_message_contains(messages, "event-driven only")
        self.assert_message_contains(messages, "raisers/intake_poll.lua")

    def test_lifecycle_forward_queue_produce_fails(self) -> None:
        tmp, root = self.make_repo()
        with tmp:
            self.write_intake_admission(root, '"devloop_intake_candidate", "devloop_ready"')
            messages = self.messages(root)

        self.assert_message_contains(messages, "must not produce lifecycle queue 'devloop_ready'")

    def test_namespaced_lifecycle_forward_queue_produce_fails(self) -> None:
        tmp, root = self.make_repo()
        with tmp:
            self.write_intake_admission(root, '"devloop_intake_candidate", "github-devloop.devloop_ready"')
            messages = self.messages(root)

        self.assert_message_contains(messages, "must not produce lifecycle queue 'github-devloop.devloop_ready'")

    def test_consensus_proposal_produce_fails(self) -> None:
        tmp, root = self.make_repo()
        with tmp:
            self.write_intake_admission(root, '"devloop_intake_candidate", "consensus.proposal"')
            messages = self.messages(root)

        self.assert_message_contains(messages, "must not produce 'consensus.proposal'")

    def test_issue_list_self_read_fails_in_production_code(self) -> None:
        tmp, root = self.make_repo()
        with tmp:
            (root / "packages" / "github-devloop-intake" / "core" / "poll.lua").write_text(
                "local function read(repo)\n  return github().issue_list(repo, 30)\nend\n",
                encoding="utf-8",
            )
            messages = self.messages(root)

        self.assert_message_contains(messages, "must not self-read GitHub issue lists")

    def test_state_marker_write_fails(self) -> None:
        tmp, root = self.make_repo()
        with tmp:
            self.write_intake_admission(
                root,
                extra='return core.state_marker("github-devloop/issue/o/r/1", "ready", "v1")',
            )
            messages = self.messages(root)

        self.assert_message_contains(messages, "must not build or write state:v1 markers")

    def test_state_marker_literal_write_fails(self) -> None:
        tmp, root = self.make_repo()
        with tmp:
            self.write_intake_admission(
                root,
                extra='return "<!-- fkst:github-devloop:state:v1 -->", "state:v1"',
            )
            messages = self.messages(root)

        self.assert_message_contains(messages, "must not build or write state:v1 markers")

    def test_comments_strings_and_tests_do_not_count_as_production_violations(self) -> None:
        tmp, root = self.make_repo()
        with tmp:
            (root / "packages" / "github-devloop-intake" / "core" / "notes.lua").write_text(
                '-- github().issue_list(repo, 30)\nlocal text = "core.state_marker(...) and state:v1"\nreturn {}\n',
                encoding="utf-8",
            )
            test_dir = root / "packages" / "github-devloop-intake" / "tests"
            test_dir.mkdir()
            (test_dir / "fixture_test.lua").write_text(
                'return { "issue_list", "state:v1" }\n',
                encoding="utf-8",
            )
            self.assertEqual(self.messages(root), [])

    def test_zero_candidate_consumer_fails(self) -> None:
        tmp, root = self.make_repo()
        with tmp:
            self.remove_default_consumer(root)
            messages = self.messages(root)

        self.assert_message_contains(messages, "non-empty subset")
        self.assert_message_contains(messages, "found none")

    def test_test_fixture_candidate_consumers_do_not_count(self) -> None:
        tmp, root = self.make_repo()
        with tmp:
            fixture = root / "packages" / "github-devloop-test-fixture" / "tests"
            fixture.mkdir(parents=True)
            (fixture / "candidate_fixture_test.lua").write_text(
                textwrap.dedent(
                    """\
                    local spec = {
                      consumes = { "github-devloop-intake.devloop_intake_candidate" },
                      produces = {},
                    }
                    return spec
                    """
                ),
                encoding="utf-8",
            )
            self.assertEqual(self.messages(root), [])

    def test_third_candidate_consumer_package_fails(self) -> None:
        tmp, root = self.make_repo()
        with tmp:
            self.write_workflow_consumer(root)
            other = root / "packages" / "github-devloop-other" / "departments" / "admission"
            other.mkdir(parents=True)
            (other / "main.lua").write_text(
                textwrap.dedent(
                    """\
                    local spec = {
                      consumes = { "github-devloop-intake.devloop_intake_candidate" },
                      produces = {},
                    }
                    return require("workflow.saga").department(spec, { act = function() end, done = function() return false end })
                    """
                ),
                encoding="utf-8",
            )
            messages = self.messages(root)

        self.assert_message_contains(messages, "non-empty subset")
        self.assert_message_contains(messages, "github-devloop-intake-default")
        self.assert_message_contains(messages, "github-devloop-workflow")
        self.assert_message_contains(messages, "github-devloop-other")


if __name__ == "__main__":
    unittest.main()
