#!/usr/bin/env python3
"""Fail-closed oracle tests for the cron acceptance workflow."""

from __future__ import annotations

import json
import pathlib
import sys
import tempfile
import unittest
from unittest import mock

sys.path.insert(0, str(pathlib.Path(__file__).parent))

import collect
import publish


class CollectTests(unittest.TestCase):
    def run_collect(self, recorded: list[str]) -> tuple[int, list[dict] | None]:
        with tempfile.TemporaryDirectory() as directory:
            candidates_path = pathlib.Path(directory) / "candidates.json"
            with mock.patch.object(collect.ledger, "read", return_value=(recorded, {})):
                with mock.patch.object(
                    sys,
                    "argv",
                    ["collect.py", "--topic", "AI Tools", "--count", "3"],
                ):
                    with mock.patch.object(collect, "CANDIDATES", candidates_path):
                        result = collect.main()
                        candidates = (
                            json.loads(candidates_path.read_text(encoding="utf-8"))
                            if candidates_path.exists()
                            else None
                        )
        return result, candidates

    def test_requires_observable_prior_ledger_suppression(self) -> None:
        result, candidates = self.run_collect([])

        self.assertEqual(result, 1)
        self.assertIsNone(candidates)

    def test_emits_only_fresh_candidates_after_suppression(self) -> None:
        result, candidates = self.run_collect(["ai-tools-0000"])

        self.assertEqual(result, 0)
        self.assertEqual(
            candidates,
            [
                {"id": "ai-tools-0001", "title": "ai-tools candidate 1"},
                {"id": "ai-tools-0002", "title": "ai-tools candidate 2"},
            ],
        )


class PublishValidationTests(unittest.TestCase):
    def setUp(self) -> None:
        self.candidates = {
            "candidate-1": {"id": "candidate-1", "title": "Candidate one"},
            "candidate-2": {"id": "candidate-2", "title": "Candidate two"},
        }
        self.valid_scores = [
            {"id": "candidate-1", "score": 8, "rationale": "Strong fit."},
            {"id": "candidate-2", "score": 4, "rationale": "Weak fit."},
        ]

    def test_accepts_complete_unique_integer_scores(self) -> None:
        publish.validate_scores(self.candidates, self.valid_scores)

    def test_rejects_empty_incomplete_duplicate_and_malformed_scores(self) -> None:
        cases = [
            [],
            self.valid_scores[:1],
            [self.valid_scores[0], self.valid_scores[0]],
            [
                {"id": "candidate-1", "score": 8.0, "rationale": "Not an integer."},
                self.valid_scores[1],
            ],
            [
                {
                    "id": "candidate-1",
                    "score": 8,
                    "rationale": "Strong fit.",
                    "extra": True,
                },
                self.valid_scores[1],
            ],
        ]

        for scores in cases:
            with self.subTest(scores=scores):
                with self.assertRaises(SystemExit):
                    publish.validate_scores(self.candidates, scores)


class PublishMainTests(unittest.TestCase):
    def run_publish(
        self, scores: list[dict], *, changed: bool = True
    ) -> tuple[int, mock.Mock]:
        candidates = [
            {"id": "candidate-1", "title": "Candidate one"},
            {"id": "candidate-2", "title": "Candidate two"},
        ]
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            candidates_path = root / "candidates.json"
            scores_path = root / "scored.json"
            candidates_path.write_text(json.dumps(candidates), encoding="utf-8")
            scores_path.write_text(json.dumps(scores), encoding="utf-8")
            ledger_write = mock.Mock(return_value=changed)
            with mock.patch.object(publish, "CANDIDATES", candidates_path):
                with mock.patch.object(publish, "SCORED", scores_path):
                    with mock.patch.object(publish.ledger, "read", return_value=([], {})):
                        with mock.patch.object(publish.ledger, "write", ledger_write):
                            with mock.patch.object(
                                sys,
                                "argv",
                                ["publish.py", "--min-score", "5"],
                            ):
                                result = publish.main()
        return result, ledger_write

    def test_requires_at_least_one_published_row(self) -> None:
        result, ledger_write = self.run_publish(
            [
                {"id": "candidate-1", "score": 4, "rationale": "Below threshold."},
                {"id": "candidate-2", "score": 3, "rationale": "Below threshold."},
            ]
        )

        self.assertEqual(result, 1)
        ledger_write.assert_not_called()

    def test_requires_a_ledger_mutation(self) -> None:
        result, ledger_write = self.run_publish(
            [
                {"id": "candidate-1", "score": 8, "rationale": "Strong fit."},
                {"id": "candidate-2", "score": 4, "rationale": "Weak fit."},
            ],
            changed=False,
        )

        self.assertEqual(result, 1)
        ledger_write.assert_called_once()

    def test_succeeds_with_valid_publication_and_commit_evidence(self) -> None:
        result, ledger_write = self.run_publish(
            [
                {"id": "candidate-1", "score": 8, "rationale": "Strong fit."},
                {"id": "candidate-2", "score": 4, "rationale": "Weak fit."},
            ]
        )

        self.assertEqual(result, 0)
        ledger_write.assert_called_once()


if __name__ == "__main__":
    unittest.main()
