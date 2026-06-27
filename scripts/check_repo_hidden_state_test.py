#!/usr/bin/env python3

from __future__ import annotations

import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import check_repo_hidden_state as hidden


class HiddenStateRatchetTest(unittest.TestCase):
    def make_repo(self) -> tuple[tempfile.TemporaryDirectory[str], Path]:
        tmp = tempfile.TemporaryDirectory()
        root = Path(tmp.name)
        (root / "migration").mkdir(parents=True)
        (root / "packages" / "github-devloop" / "core").mkdir(parents=True)
        (root / "packages" / "github-devloop-pr" / "core").mkdir(parents=True)
        (root / "libraries" / "devloop").mkdir(parents=True)
        (root / "migration" / "hidden-state.allowlist").write_text(
            "github-devloop|ready|dependency-gate|implementing|issue=#1595|why=existing behavioral debt\n",
            encoding="utf-8",
        )
        (root / "libraries" / "devloop" / "hidden_state_conformance.lua").write_text(
            "\n".join(
                (
                    "local ALLOWLIST_PATH = 'migration/hidden-state.allowlist'",
                    "local function build_fixture() end",
                    "local function behavioral_errors() end",
                    "local non_durable_advance = true",
                    "local positive = 'positive poll fixture'",
                    "local negative = 'negative poll fixture'",
                    "core.replay_from_table()",
                )
            ),
            encoding="utf-8",
        )
        (root / "packages" / "github-devloop" / "core" / "hidden_state_conformance.lua").write_text(
            'return require("devloop.hidden_state_conformance")\n',
            encoding="utf-8",
        )
        for package in ("github-devloop", "github-devloop-pr"):
            (root / "packages" / package / "core.lua").write_text(
                'require("devloop.hidden_state_conformance").install(M)\n',
                encoding="utf-8",
            )
            (root / "packages" / package / "core" / "span_conformance.lua").write_text(
                "core.hidden_state_conformance_errors()\n",
                encoding="utf-8",
            )
        (root / "libraries" / "devloop" / "replayer.lua").write_text("return {}\n", encoding="utf-8")
        return tmp, root

    def test_behavioral_harness_shape_passes_without_base_enforcement(self) -> None:
        tmp, root = self.make_repo()
        with tmp:
            self.assertEqual(hidden.repository_messages(root, enforce_base=False), [])

    def test_allowlist_line_requires_tracking_issue_and_why(self) -> None:
        tmp, root = self.make_repo()
        with tmp:
            (root / "migration" / "hidden-state.allowlist").write_text(
                "github-devloop|ready|dependency-gate|implementing|why=missing issue\n",
                encoding="utf-8",
            )
            with self.assertRaises(ValueError):
                hidden.repository_messages(root, enforce_base=False)

    def test_capability_guard_file_fails(self) -> None:
        tmp, root = self.make_repo()
        with tmp:
            (root / "libraries" / "devloop" / "replayer_hidden_state.lua").write_text("return {}\n", encoding="utf-8")
            messages = hidden.repository_messages(root, enforce_base=False)

        self.assertTrue(any("rejected capability" in message for message in messages), messages)

    def test_capability_tokens_in_conformance_fail(self) -> None:
        tmp, root = self.make_repo()
        with tmp:
            path = root / "libraries" / "devloop" / "hidden_state_conformance.lua"
            path.write_text(path.read_text(encoding="utf-8") + "\nlocal x = 'safe_entity_view'\n", encoding="utf-8")
            messages = hidden.repository_messages(root, enforce_base=False)

        self.assertTrue(any("safe_entity_view" in message for message in messages), messages)

    def test_pr_package_must_install_hidden_state_conformance(self) -> None:
        tmp, root = self.make_repo()
        with tmp:
            (root / "packages" / "github-devloop-pr" / "core.lua").write_text("return M\n", encoding="utf-8")
            messages = hidden.repository_messages(root, enforce_base=False)

        self.assertTrue(any("github-devloop-pr/core.lua" in message for message in messages), messages)

    def test_allowlist_growth_fails_when_base_is_known(self) -> None:
        key = hidden.HiddenStateKey("github-devloop", "ready", "dependency-gate", "implementing")
        grown = hidden.HiddenStateKey("github-devloop", "awaiting-pr", "child-state", "merged")
        messages = hidden.ratchet_messages({key, grown}, {key})

        self.assertEqual(len(messages), 1)
        self.assertIn("grows migration/hidden-state.allowlist relative to dev", messages[0])


if __name__ == "__main__":
    unittest.main()
