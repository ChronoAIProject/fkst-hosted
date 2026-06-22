#!/usr/bin/env python3
"""Unit tests for the github-devloop competence gate."""

from __future__ import annotations

import importlib.util
import json
import shutil
import sys
import tempfile
import unittest
from pathlib import Path


def load_competence_gate():
    path = Path(__file__).with_name("competence_gate.py")
    spec = importlib.util.spec_from_file_location("competence_gate", path)
    if spec is None or spec.loader is None:
        raise RuntimeError("could not load competence_gate.py")
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


competence_gate = load_competence_gate()


class CompetenceGateTest(unittest.TestCase):
    def repo_root(self) -> Path:
        return Path(__file__).resolve().parents[1]

    def obligations(self) -> dict:
        return competence_gate.load_obligations(self.repo_root())

    def test_classifier_marks_restart_contract_as_l3(self) -> None:
        classification = competence_gate.classify_paths(
            ["libraries/devloop/restart_responsibility_contract.lua"],
            self.obligations(),
        )

        self.assertEqual(classification.level, "L3")
        self.assertIn("liveness-responsibility-maps", classification.surfaces)

    def test_classifier_marks_scripts_as_l2(self) -> None:
        classification = competence_gate.classify_paths(
            ["scripts/check_repo.py"],
            self.obligations(),
        )

        self.assertEqual(classification.level, "L2")
        self.assertEqual(classification.surfaces, ())

    def test_obligations_validate_seed_corpus(self) -> None:
        errors = competence_gate.validate_obligations(self.obligations())

        self.assertEqual(errors, [])

    def test_l3_gate_passes_current_seed_evidence(self) -> None:
        rc, report = competence_gate.run(
            self.repo_root(),
            ["libraries/devloop/restart_responsibility_contract.lua"],
        )

        self.assertEqual(rc, 0, report["errors"])
        self.assertEqual(report["classification"]["level"], "L3")
        self.assertEqual(report["metrics"]["challenge_recall"], 1)
        self.assertEqual(report["metrics"]["bug_class_recall"], 1)
        self.assertEqual(report["metrics"]["false_reject_rate"], 0)
        self.assertEqual(report["metrics"]["mutant_kill_rate"], 1)

    def test_removing_invariant_six_mutation_requirement_fails(self) -> None:
        obligations = self.obligations()
        for item in obligations["obligations"]:
            if item["id"] == "006-ready-dependency-partition-boundary":
                item.pop("mutation_required", None)

        errors = competence_gate.validate_obligations(obligations)

        self.assertIn("invariant #6 obligation must require mutation enforcement", errors)

    def test_invariant_six_has_production_evidence(self) -> None:
        obligations = self.obligations()
        invariant = next(
            item
            for item in obligations["obligations"]
            if item["id"] == "006-ready-dependency-partition-boundary"
        )

        self.assertEqual(
            invariant["production_evidence_path"],
            "libraries/devloop/restart_responsibility_contract.lua",
        )
        source = (self.repo_root() / invariant["production_evidence_path"]).read_text(encoding="utf-8")
        self.assertIn(invariant["production_evidence"], source)

    def test_removing_negative_control_fails(self) -> None:
        obligations = self.obligations()
        obligations["obligations"] = [
            item
            for item in obligations["obligations"]
            if item["id"] != "007-partial-write-idempotency-completeness"
        ]

        errors = competence_gate.validate_obligations(obligations)

        self.assertTrue(any("obligations must list exactly 7" in error for error in errors))
        self.assertTrue(any("negative controls missing" in error for error in errors))

    def test_run_script_check_invokes_competence_gate(self) -> None:
        source = (self.repo_root() / "scripts" / "run.sh").read_text(encoding="utf-8")

        self.assertIn('python3 -B "$ROOT/scripts/competence_gate.py"', source)
        self.assertIn('--base-ref "$competence_base_ref"', source)
        self.assertIn("competence_gate_base_ref()", source)


if __name__ == "__main__":
    unittest.main()
