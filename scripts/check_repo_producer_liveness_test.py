#!/usr/bin/env python3

import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import check_repo_producer_liveness as p


class ProducerLivenessAdversarialFixtureTest(unittest.TestCase):
    def test_fixture_tokens_are_name_based(self) -> None:
        source = (
            'function test_fire_raiser_audit_poll_busy_overdue_produces_issue_create_request()\n'
            ' local trace = t.fire_raiser("audit_poll")\n'
            ' t.eq(trace.consumer_result.status, "accepted")\n'
            "end\n"
        )

        self.assertEqual(p.covered_raiser_tests_in_source(source)["busy_overdue"], {"audit_poll"})

    def test_mock_idle_only_gated_producer_fails_busy_liveness_requirement(self) -> None:
        raiser = p.ProducerRaiser("archaudit", "audit_poll", "packages/archaudit/raisers/audit_poll.lua", ("archaudit_tick",))
        contract = p.ProducerLivenessContract(
            package="archaudit",
            producer_id="archaudit.audit",
            trigger_source="archaudit_tick",
            runtime_gate="idle_when_not_overdue",
            adversarial_fixture="busy_overdue",
        )
        happy_path_only = {"archaudit": {"audit_poll"}}
        fixture_coverage = {"archaudit": {"idle_due": {"audit_poll"}, "": {"audit_poll"}}}

        messages = p.ratchet_messages(
            {raiser},
            happy_path_only,
            set(),
            set(),
            fixture_coverage,
            {contract},
        )

        self.assertEqual(len(messages), 1)
        self.assertIn("runtime-gated by idle_when_not_overdue", messages[0])
        self.assertIn("busy_overdue", messages[0])

    def test_declared_busy_fixture_satisfies_gated_producer(self) -> None:
        raiser = p.ProducerRaiser("archaudit", "audit_poll", "packages/archaudit/raisers/audit_poll.lua", ("archaudit_tick",))
        contract = p.ProducerLivenessContract(
            package="archaudit",
            producer_id="archaudit.audit",
            trigger_source="archaudit_tick",
            runtime_gate="idle_when_not_overdue",
            adversarial_fixture="busy_overdue",
        )
        coverage = {"archaudit": {"audit_poll"}}
        fixture_coverage = {"archaudit": {"busy_overdue": {"audit_poll"}, "": {"audit_poll"}}}

        self.assertEqual(p.ratchet_messages({raiser}, coverage, set(), set(), fixture_coverage, {contract}), [])


if __name__ == "__main__":
    unittest.main()
