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
        (root / "packages" / "github-devloop-intake" / "core").mkdir(parents=True)
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

    def write_default_consumer(self, root: Path, queue: str = "github-devloop-intake.devloop_intake_candidate") -> None:
        (root / "packages" / "github-devloop-intake-default" / "departments" / "intake_judge" / "main.lua").write_text(
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

    def messages(self, root: Path) -> list[str]:
        return intake_routing.repository_messages(root)

    def assert_message_contains(self, messages: list[str], expected: str) -> None:
        self.assertTrue(
            any(expected in message for message in messages),
            f"expected message containing {expected!r}, got {messages!r}",
        )

    def test_current_thin_intake_shape_passes(self) -> None:
        tmp, root = self.make_repo()
        with tmp:
            self.assertEqual(self.messages(root), [])

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
            self.write_default_consumer(root, "github-devloop-intake.other_candidate")
            messages = self.messages(root)

        self.assert_message_contains(messages, "expected exactly one package to consume github-devloop-intake.devloop_intake_candidate")
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

    def test_multiple_candidate_consumer_packages_fail(self) -> None:
        tmp, root = self.make_repo()
        with tmp:
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

        self.assert_message_contains(messages, "expected exactly one package to consume github-devloop-intake.devloop_intake_candidate")
        self.assert_message_contains(messages, "github-devloop-intake-default")
        self.assert_message_contains(messages, "github-devloop-other")


if __name__ == "__main__":
    unittest.main()
