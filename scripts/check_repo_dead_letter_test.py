#!/usr/bin/env python3
"""Tests for the reliable-consumer dead-letter topology guard."""

from __future__ import annotations

import tempfile
import unittest
from pathlib import Path

import check_repo_dead_letter
import check_repo_ingress


class DeadLetterConsumerGuardTest(unittest.TestCase):
    def repository_messages(self, files: dict[str, str]) -> list[str]:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            for rel_path, source in files.items():
                path = root / rel_path
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text(source, encoding="utf-8")

            return check_repo_dead_letter.repository_messages(root)

    def test_reliable_consumer_requires_package_dead_letter_consumer(self) -> None:
        messages = self.repository_messages({
            "packages/demo/departments/worker/main.lua": 'M.spec = { consumes = { "work" } }\n',
        })

        self.assertEqual(len(messages), 1)
        self.assertIn("packages/demo consumes reliable queues but has no department consuming `dead_letter`", messages[0])
        self.assertIn("work", messages[0])

    def test_ephemeral_only_consumer_does_not_require_dead_letter_consumer(self) -> None:
        self.assertEqual(
            self.repository_messages({
                "packages/demo/departments/cache/main.lua": 'M.spec = { consumes = { "cache_seed" }, ephemeral = { "cache_seed" } }\n',
            }),
            [],
        )

    def test_spec_queues_parses_ephemeral_field(self) -> None:
        source = 'M.spec = { consumes = { "cache_seed" }, ephemeral = { "cache_seed" } }\n'

        self.assertEqual(check_repo_ingress.spec_queues(source, "ephemeral"), {"cache_seed"})

    def test_package_dead_letter_consumer_satisfies_reliable_consumer(self) -> None:
        self.assertEqual(
            self.repository_messages({
                "packages/demo/departments/worker/main.lua": 'M.spec = { consumes = { "work" } }\n',
                "packages/demo/departments/dead_letter/main.lua": 'M.spec = { consumes = { "dead_letter" } }\n',
            }),
            [],
        )


if __name__ == "__main__":
    unittest.main()
