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


if __name__ == "__main__":
    unittest.main()
