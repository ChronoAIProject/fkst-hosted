#!/usr/bin/env python3
"""Tests for the cross-package integration coverage ratchet."""

from __future__ import annotations

import importlib.util
import json
import sys
import tempfile
import textwrap
import unittest
from pathlib import Path


def load_module():
    path = Path(__file__).with_name("check_repo_integration_coverage.py")
    spec = importlib.util.spec_from_file_location("check_repo_integration_coverage", path)
    if spec is None or spec.loader is None:
        raise RuntimeError("could not load check_repo_integration_coverage.py")
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


integration_coverage = load_module()


def write(path: Path, content: str) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(textwrap.dedent(content), encoding="utf-8")


class IntegrationCoverageRatchetTest(unittest.TestCase):
    def make_repo(self) -> tuple[tempfile.TemporaryDirectory[str], Path]:
        tmp = tempfile.TemporaryDirectory()
        root = Path(tmp.name)
        write(
            root / "packages" / "autochrono" / "departments" / "propose" / "main.lua",
            """\
            local spec = {
              consumes = { "issue" },
              produces = { "consensus.proposal" },
            }
            return { spec = spec }
            """,
        )
        write(
            root / "packages" / "consensus" / "departments" / "decide" / "main.lua",
            """\
            local spec = {
              consumes = { "proposal" },
              produces = { "consensus_reached" },
            }
            return { spec = spec }
            """,
        )
        write(
            root / "packages" / "autochrono" / "departments" / "reply" / "main.lua",
            """\
            local spec = {
              consumes = { "consensus.consensus_reached" },
              produces = { "reply" },
            }
            return { spec = spec }
            """,
        )
        (root / "migration").mkdir()
        return tmp, root

    def test_cross_package_edges_use_actual_producer_package_not_queue_prefix(self) -> None:
        tmp, root = self.make_repo()
        with tmp:
            edges = integration_coverage.cross_package_edges(root)

        self.assertIn("consensus.proposal -> consensus.decide", edges)
        self.assertIn("consensus.consensus_reached -> autochrono.reply", edges)
        self.assertNotIn("autochrono.issue -> autochrono.propose", edges)

    def test_observed_edges_are_static_assert_covers_strings(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            write(
                root / "packages" / "autochrono" / "tests" / "run_graph_smoke_test.lua",
                """\
                local graph = require("testkit.graph")
                local function test()
                  graph.assert_covers(trace, {
                    "consensus.proposal -> consensus.decide",
                    "consensus.consensus_reached -> autochrono.reply",
                  })
                end
                """,
            )

            observed = integration_coverage.observed_edges(root)

        self.assertEqual(
            observed,
            {
                "consensus.proposal -> consensus.decide",
                "consensus.consensus_reached -> autochrono.reply",
            },
        )

    def test_ratchet_messages_enforce_uncovered_and_stale_allowlist_entries(self) -> None:
        edges = {
            "consensus.proposal -> consensus.decide",
            "consensus.consensus_reached -> autochrono.reply",
        }
        observed = {"consensus.proposal -> consensus.decide"}
        allowlist = {"consensus.consensus_reached -> autochrono.reply", "stale.queue -> stale.consumer"}

        messages = integration_coverage.ratchet_messages(edges, observed, allowlist)

        joined = "\n".join(messages)
        self.assertIn("stale: stale.queue -> stale.consumer no longer exists", joined)
        self.assertNotIn("new uncovered cross-package edge consensus.consensus_reached", joined)

    def test_repository_messages_pass_when_allowlist_matches_current_uncovered(self) -> None:
        tmp, root = self.make_repo()
        with tmp:
            write(
                root / "packages" / "autochrono" / "tests" / "run_graph_smoke_test.lua",
                """\
                graph.assert_covers(trace, {
                  "consensus.proposal -> consensus.decide",
                })
                """,
            )
            (root / integration_coverage.ALLOWLIST).write_text(
                json.dumps({"edge": "consensus.consensus_reached -> autochrono.reply", "reason": "baseline"}) + "\n",
                encoding="utf-8",
            )

            messages = integration_coverage.repository_messages(root)

        self.assertEqual(messages, [])

    def test_host_mode_filters_platform_owned_edges_and_reports_owner_scope(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            base = Path(tmp)
            host = base / "host"
            platform = base / "platform"
            write(
                platform / "packages" / "platform-producer" / "departments" / "emit" / "main.lua",
                """\
                local spec = {
                  consumes = {},
                  produces = { "platform_event" },
                }
                return { spec = spec }
                """,
            )
            write(
                platform / "packages" / "platform-consumer" / "departments" / "take" / "main.lua",
                """\
                local spec = {
                  consumes = { "platform-producer.platform_event" },
                  produces = {},
                }
                return { spec = spec }
                """,
            )
            write(
                host / ".fkst" / "local-packages" / "site-board" / "departments" / "take" / "main.lua",
                """\
                local spec = {
                  consumes = { "platform-producer.platform_event" },
                  produces = {},
                }
                return { spec = spec }
                """,
            )

            report = integration_coverage.edge_report(host, platform_root=platform)
            messages = integration_coverage.repository_messages(host, platform_root=platform)

        edge_ids = {entry["edge_id"] for entry in report}
        self.assertIn("platform-producer.platform_event -> site-board.take", edge_ids)
        self.assertNotIn("platform-producer.platform_event -> platform-consumer.take", edge_ids)
        self.assertEqual(
            [entry["owner_scope"] for entry in report if entry["edge_id"] == "platform-producer.platform_event -> site-board.take"],
            ["host-owned"],
        )
        self.assertEqual(len(messages), 1)
        self.assertIn("platform-producer.platform_event -> site-board.take", messages[0])

    def test_exclusions_are_typed_and_stale_entries_fail(self) -> None:
        edges = {"consensus.proposal -> consensus.decide"}
        messages = integration_coverage.ratchet_messages(
            edges,
            observed=set(),
            allowlist=set(),
            exclusions={
                "consensus.proposal -> consensus.decide": integration_coverage.Exclusion(
                    edge="consensus.proposal -> consensus.decide",
                    reason="host-owned permanent gap",
                    owner="platform",
                    review_by="2026-12-31",
                ),
                "stale.queue -> stale.consumer": integration_coverage.Exclusion(
                    edge="stale.queue -> stale.consumer",
                    reason="old gap",
                    owner="platform",
                    review_by="2026-12-31",
                ),
            },
        )

        joined = "\n".join(messages)
        self.assertIn("stale: excluded edge stale.queue -> stale.consumer no longer exists", joined)
        self.assertNotIn("new uncovered cross-package edge consensus.proposal", joined)

    def test_load_exclusions_requires_owner_reason_and_review_by(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "integration-edge-coverage.exclusions"
            path.write_text(
                json.dumps(
                    {
                        "edge": "consensus.proposal -> consensus.decide",
                        "reason": "missing owner",
                        "review_by": "2026-12-31",
                    }
                )
                + "\n",
                encoding="utf-8",
            )

            with self.assertRaisesRegex(ValueError, "owner is required"):
                integration_coverage.load_exclusions(path)

    def test_report_marks_allowlisted_excluded_and_unlisted_statuses(self) -> None:
        edges = {
            "consensus.proposal -> consensus.decide",
            "consensus.consensus_reached -> autochrono.reply",
            "autochrono.reply -> github-autochrono.outbound_glue",
            "github-proxy.github_entity_changed -> github-autochrono.inbound_glue",
        }
        observed = {"consensus.proposal -> consensus.decide"}

        report = integration_coverage.report_for_edges(
            edges,
            observed,
            allowlist={"consensus.consensus_reached -> autochrono.reply"},
            exclusions={
                "autochrono.reply -> github-autochrono.outbound_glue": integration_coverage.Exclusion(
                    edge="autochrono.reply -> github-autochrono.outbound_glue",
                    reason="documented host-owned gap",
                    owner="platform",
                    review_by="2026-12-31",
                )
            },
            platform_packages=set(),
        )

        by_edge = {entry["edge_id"]: entry for entry in report}
        self.assertEqual(by_edge["consensus.proposal -> consensus.decide"]["status"], "covered")
        self.assertEqual(
            by_edge["consensus.consensus_reached -> autochrono.reply"]["status"],
            "uncovered-allowlisted",
        )
        self.assertEqual(by_edge["autochrono.reply -> github-autochrono.outbound_glue"]["status"], "excluded")
        self.assertEqual(
            by_edge["github-proxy.github_entity_changed -> github-autochrono.inbound_glue"]["status"],
            "uncovered-UNLISTED",
        )
        self.assertEqual(by_edge["consensus.proposal -> consensus.decide"]["queue"], "consensus.proposal")
        self.assertEqual(by_edge["consensus.proposal -> consensus.decide"]["producer_pkg"], "consensus")


if __name__ == "__main__":
    unittest.main()
