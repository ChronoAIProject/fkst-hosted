#!/usr/bin/env python3
"""Unit tests for the G-MONOTONE-GATE repository guard."""

from __future__ import annotations

import importlib.util
import sys
import tempfile
import textwrap
import unittest
from pathlib import Path


def load_module():
    path = Path(__file__).with_name("check_repo_monotone_gate.py")
    spec = importlib.util.spec_from_file_location("check_repo_monotone_gate", path)
    if spec is None or spec.loader is None:
        raise RuntimeError("could not load check_repo_monotone_gate.py")
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


monotone = load_module()


class MonotoneGateRatchetTest(unittest.TestCase):
    def test_undeclared_cursor_gate_is_flagged(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            target = root / "packages" / "github-devloop" / "departments" / "observe_issue" / "main.lua"
            target.parent.mkdir(parents=True)
            target.write_text(
                textwrap.dedent(
                    """\
                    local function planted_gate(comments, proposal_id)
                      local current = core.current_state(comments, proposal_id)
                      return current ~= nil and current.state == "pr-open"
                    end
                    """
                ),
                encoding="utf-8",
            )
            (root / "migration").mkdir()
            (root / monotone.MANIFEST).write_text(
                "# no declared monotone surfaces; broad scan must still catch the raw cursor read\n",
                encoding="utf-8",
            )
            (root / monotone.ALLOWLIST).write_text("", encoding="utf-8")

            messages = monotone.repository_messages(root, enforce_base=False)

        joined = "\n".join(messages)
        self.assertIn("planted_gate cursor-read current_state(", joined)
        self.assertIn("planted_gate state-equality pr-open", joined)
        self.assertIn("unclassified transient lifecycle cursor read", joined)

    def test_undeclared_cursor_gate_in_split_devloop_package_is_flagged(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            target = root / "packages" / "github-devloop-integration" / "departments" / "pr_freshness_scan" / "main.lua"
            target.parent.mkdir(parents=True)
            target.write_text(
                textwrap.dedent(
                    """\
                    local function planted_integration_gate(comments, proposal_id)
                      local current = core.current_entity_state(comments, proposal_id)
                      return current ~= nil and current.state == "reviewing"
                    end
                    """
                ),
                encoding="utf-8",
            )
            (root / "migration").mkdir()
            (root / monotone.MANIFEST).write_text(
                "# no declared monotone surfaces; github-devloop* split packages must be scanned\n",
                encoding="utf-8",
            )
            (root / monotone.ALLOWLIST).write_text("", encoding="utf-8")

            messages = monotone.repository_messages(root, enforce_base=False)

        joined = "\n".join(messages)
        self.assertIn("packages/github-devloop-integration/departments/pr_freshness_scan/main.lua", joined)
        self.assertIn("planted_integration_gate cursor-read current_entity_state(", joined)
        self.assertIn("planted_integration_gate state-equality reviewing", joined)
        self.assertIn("unclassified transient lifecycle cursor read", joined)

    def test_reached_gate_without_cursor_passes(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            target = root / "packages" / "github-devloop" / "departments" / "observe_issue" / "main.lua"
            target.parent.mkdir(parents=True)
            target.write_text(
                textwrap.dedent(
                    """\
                    local function planted_gate(comments, proposal_id)
                      return core.reached(comments, proposal_id, "pr-open", {
                        domain = "github-devloop-pr",
                      })
                    end
                    """
                ),
                encoding="utf-8",
            )
            (root / "migration").mkdir()
            (root / monotone.MANIFEST).write_text(
                "# no declared monotone surfaces; reached() has no raw cursor read\n",
                encoding="utf-8",
            )
            (root / monotone.ALLOWLIST).write_text("", encoding="utf-8")

            messages = monotone.repository_messages(root, enforce_base=False)

        self.assertEqual(messages, [])

    def test_monotone_signature_requires_implementation_body_to_use_accessor(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            transition = root / "packages" / "github-devloop" / "core" / "restart" / "transitions" / "synthetic.lua"
            implementation = root / "packages" / "github-devloop" / "core" / "synthetic_gate.lua"
            transition.parent.mkdir(parents=True)
            implementation.parent.mkdir(parents=True, exist_ok=True)
            transition.write_text(
                textwrap.dedent(
                    """\
                    return {
                      responsibility_signature = responsibility_signature({
                        state_kind = "gate",
                        gate_kind = "monotone_milestone",
                        milestone_accessor = "std.devloop_state.reached",
                        milestone_implementation = "packages/github-devloop/core/synthetic_gate.lua:M.synthetic_gate",
                        milestone = "pr-open",
                        milestone_domain = "github-devloop-pr",
                      }),
                    }
                    """
                ),
                encoding="utf-8",
            )
            implementation.write_text(
                textwrap.dedent(
                    """\
                    function M.synthetic_gate(comments, proposal_id)
                      local current = M.current_state(comments, proposal_id)
                      return current.state == "pr-open"
                    end
                    """
                ),
                encoding="utf-8",
            )
            (root / "migration").mkdir()
            (root / monotone.MANIFEST).write_text("", encoding="utf-8")
            (root / monotone.ALLOWLIST).write_text(
                "packages/github-devloop/core/synthetic_gate.lua|M.synthetic_gate|cursor-read|current_state(|line=2|issue=#1310|why=classified current routing/decision read; migrate only when it is a monotone milestone gate\n"
                "packages/github-devloop/core/synthetic_gate.lua|M.synthetic_gate|state-equality|pr-open|line=3|issue=#1310|why=classified current routing/decision read; migrate only when it is a monotone milestone gate\n",
                encoding="utf-8",
            )

            messages = monotone.repository_messages(root, enforce_base=False)

        joined = "\n".join(messages)
        self.assertIn("implementation packages/github-devloop/core/synthetic_gate.lua:M.synthetic_gate does not reference std.devloop_state.reached", joined)
        self.assertIn("reads a transient cursor inside monotone_milestone implementation", joined)


if __name__ == "__main__":
    unittest.main()
