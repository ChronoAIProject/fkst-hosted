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


def issue_1310(line: int) -> str:
    return (
        f"line={line}|issue=#1310|"
        "why=classified current routing/decision read; migrate only when it is a monotone milestone gate"
    )


def allowline(path: str, surface: str, kind: str, token: str, line: int) -> str:
    return f"{path}|{surface}|{kind}|{token}|{issue_1310(line)}\n"


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
                        milestone_accessor = "devloop.state.reached",
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
        self.assertIn("implementation packages/github-devloop/core/synthetic_gate.lua:M.synthetic_gate does not reference devloop.state.reached", joined)
        self.assertIn("reads a transient cursor inside monotone_milestone implementation", joined)

    def test_ops_package_extraction_does_not_grow_existing_allowlist_debt(self) -> None:
        current = {
            monotone.Violation(
                "packages/github-devloop-ops/departments/observability/census.lua",
                "put_issue_entity",
                "cursor-read",
                "current_state(",
                18,
            )
        }
        allowlist = set(current)
        base = {
            monotone.Violation(
                "packages/github-devloop/departments/observability/census.lua",
                "put_issue_entity",
                "cursor-read",
                "current_state(",
                18,
            )
        }

        self.assertEqual(monotone.ratchet_messages(current, allowlist, base), [])

    def test_implement_worktree_extraction_does_not_grow_existing_allowlist_debt(self) -> None:
        current = {
            monotone.Violation(
                "packages/github-devloop/departments/implement/main.lua",
                "precheck_implementation_write_gate",
                "cursor-read",
                "current_state(",
                499,
            )
        }
        original_debt = monotone.Violation(
            "packages/github-devloop/departments/implement/main.lua",
            "precheck_implementation_write_gate",
            "cursor-read",
            "current_state(",
            602,
        )
        allowlist = {original_debt}
        base = {
            original_debt
        }

        self.assertEqual(monotone.ratchet_messages(current, allowlist, base), [])

    def test_replayer_hidden_state_extraction_does_not_grow_existing_allowlist_debt(self) -> None:
        current = {
            monotone.Violation(
                "libraries/devloop/replayer.lua",
                "require_marker_fact",
                "state-equality",
                "implementing",
                293,
            )
        }
        original_debt = monotone.Violation(
            "libraries/devloop/replayer.lua",
            "require_marker_fact",
            "state-equality",
            "implementing",
            287,
        )
        allowlist = {original_debt}
        base = {original_debt}

        self.assertEqual(monotone.ratchet_messages(current, allowlist, base), [])

    def test_v2_blank_lines_above_findings_do_not_create_growth(self) -> None:
        current = {
            monotone.Violation(
                "packages/github-devloop/core/synthetic_gate.lua",
                "planted_gate",
                "cursor-read",
                "current_state(",
                22,
            ),
            monotone.Violation(
                "packages/github-devloop/core/synthetic_gate.lua",
                "planted_gate",
                "state-equality",
                "pr-open",
                23,
            ),
        }
        allowlist = {
            monotone.Violation(
                "packages/github-devloop/core/synthetic_gate.lua",
                "planted_gate",
                "cursor-read",
                "current_state(",
                2,
            ),
            monotone.Violation(
                "packages/github-devloop/core/synthetic_gate.lua",
                "planted_gate",
                "state-equality",
                "pr-open",
                3,
            ),
        }

        self.assertEqual(monotone.ratchet_messages(current, allowlist, allowlist), [])

    def test_v2_duplicate_raw_read_under_existing_key_is_growth(self) -> None:
        key = (
            "packages/github-devloop/core/synthetic_gate.lua",
            "planted_gate",
            "cursor-read",
            "current_state(",
        )
        current = [
            monotone.Violation(*key, 12),
            monotone.Violation(*key, 12),
        ]
        allowlist = [monotone.Violation(*key, 12)]

        messages = monotone.ratchet_messages(current, allowlist, allowlist)

        joined = "\n".join(messages)
        self.assertIn("grows monotone-gate debt relative to dev", joined)
        self.assertIn("current_lines=[12, 12]", joined)

    def test_v2_install_m_to_typed_c_surface_rename_is_not_growth(self) -> None:
        path = "packages/github-devloop/core/synthetic_gate.lua"
        allowlist = [
            monotone.Violation(path, "M.foo", "state-equality", "ready", 12),
        ]
        current = [
            monotone.Violation(path, "C.foo", "state-equality", "ready", 80),
        ]

        messages = monotone.ratchet_messages(current, allowlist, allowlist)

        self.assertEqual(messages, [])

    def test_v2_new_read_after_surface_canonicalization_still_grows(self) -> None:
        path = "packages/github-devloop/core/synthetic_gate.lua"
        allowlist = [
            monotone.Violation(path, "M.foo", "state-equality", "ready", 12),
        ]
        current = [
            monotone.Violation(path, "C.foo", "state-equality", "ready", 80),
            monotone.Violation(path, "C.bar", "state-equality", "open", 81),
        ]

        messages = monotone.ratchet_messages(current, allowlist, allowlist)

        joined = "\n".join(messages)
        self.assertIn("C.bar state-equality open", joined)
        self.assertIn("grows monotone-gate debt relative to dev", joined)

    def test_v2_second_identical_read_after_surface_canonicalization_still_grows(self) -> None:
        path = "packages/github-devloop/core/synthetic_gate.lua"
        allowlist = [
            monotone.Violation(path, "M.foo", "state-equality", "ready", 12),
        ]
        current = [
            monotone.Violation(path, "C.foo", "state-equality", "ready", 80),
            monotone.Violation(path, "C.foo", "state-equality", "ready", 81),
        ]

        messages = monotone.ratchet_messages(current, allowlist, allowlist)

        joined = "\n".join(messages)
        self.assertIn("C.foo state-equality ready", joined)
        self.assertIn("current_lines=[80, 81]", joined)
        self.assertIn("grows monotone-gate debt relative to dev", joined)

    def test_v2_cross_file_surface_rename_still_grows(self) -> None:
        allowlist = [
            monotone.Violation(
                "packages/github-devloop/core/synthetic_gate.lua",
                "M.foo",
                "state-equality",
                "ready",
                12,
            ),
        ]
        current = [
            monotone.Violation(
                "packages/github-devloop/core/moved_gate.lua",
                "C.foo",
                "state-equality",
                "ready",
                80,
            ),
        ]

        messages = monotone.ratchet_messages(current, allowlist, allowlist)

        joined = "\n".join(messages)
        self.assertIn("packages/github-devloop/core/moved_gate.lua:80", joined)
        self.assertIn("grows monotone-gate debt relative to dev", joined)

    def test_v2_bare_local_surface_canonicalizes_to_itself(self) -> None:
        violation = monotone.Violation(
            "packages/github-devloop/core/synthetic_gate.lua",
            "helper_fn",
            "state-equality",
            "ready",
            12,
        )

        self.assertEqual(violation.canonical_surface(), "helper_fn")

    def test_v2_removed_raw_read_passes_and_reports_shrink_opportunity(self) -> None:
        key = (
            "packages/github-devloop/core/synthetic_gate.lua",
            "planted_gate",
            "cursor-read",
            "current_state(",
        )
        current = [monotone.Violation(*key, 12)]
        allowlist = [
            monotone.Violation(*key, 12),
            monotone.Violation(*key, 13),
        ]

        messages = monotone.ratchet_messages(current, allowlist, allowlist)

        joined = "\n".join(messages)
        self.assertIn("monotone-gate debt count shrank", joined)
        self.assertIn("current_lines=[12]", joined)
        self.assertNotIn("grows monotone-gate", joined)

    def test_v2_resolved_devloop_alias_matches_old_m_surface_without_growth(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            target = root / "packages" / "github-devloop" / "core" / "synthetic_gate.lua"
            target.parent.mkdir(parents=True)
            target.write_text(
                textwrap.dedent(
                    """\
                    local typed = require("devloop.state")

                    local function planted_gate(comments, proposal_id)
                      local current = typed.current_state(comments, proposal_id)
                      return current ~= nil and current.state == "pr-open"
                    end
                    """
                ),
                encoding="utf-8",
            )
            (root / "migration").mkdir()
            (root / monotone.MANIFEST).write_text("", encoding="utf-8")
            (root / monotone.ALLOWLIST).write_text(
                allowline("packages/github-devloop/core/synthetic_gate.lua", "planted_gate", "cursor-read", "M.current_state(", 4)
                + allowline("packages/github-devloop/core/synthetic_gate.lua", "planted_gate", "state-equality", "pr-open", 5),
                encoding="utf-8",
            )

            messages = monotone.repository_messages(root, enforce_base=False)

        self.assertEqual(messages, [])

    def test_cursor_declaration_line_is_not_a_read_when_aliases_enable_m_prefix(self) -> None:
        source = textwrap.dedent(
            """\
            local typed = require("devloop.state")

            function M.current_entity_state(a, b)
              return M.current_state(a, b)
            end
            """
        )

        violations = monotone.source_violations("libraries/devloop/entity.lua", source)

        labels = [violation.label() for violation in violations]
        self.assertEqual(labels, ["libraries/devloop/entity.lua:4 M.current_entity_state cursor-read current_state("])

    def test_cursor_declaration_line_is_shift_insensitive(self) -> None:
        body = textwrap.dedent(
            """\
            local typed = require("devloop.state")

            function M.current_entity_state(a, b)
              return M.current_state(a, b)
            end
            """
        )
        baseline = [
            violation.canonical_key()
            for violation in monotone.source_violations("libraries/devloop/entity.lua", body)
        ]

        for inserted in (
            "\n",
            "\n".join(f'local dep_{index} = require("devloop.dep_{index}")' for index in range(20)) + "\n",
        ):
            shifted = inserted + body
            shifted_keys = [
                violation.canonical_key()
                for violation in monotone.source_violations("libraries/devloop/entity.lua", shifted)
            ]
            self.assertEqual(shifted_keys, baseline)

    def test_genuine_current_entity_state_call_is_still_detected(self) -> None:
        source = textwrap.dedent(
            """\
            local typed = require("devloop.state")

            local function planted_gate(x, y)
              local s = M.current_entity_state(x, y)
              return s
            end
            """
        )

        violations = monotone.source_violations("libraries/devloop/entity.lua", source)

        labels = [violation.label() for violation in violations]
        self.assertEqual(labels, ["libraries/devloop/entity.lua:4 planted_gate cursor-read current_entity_state("])

    def test_v2_state_token_change_is_different_key_and_flagged(self) -> None:
        current = {
            monotone.Violation(
                "packages/github-devloop/core/synthetic_gate.lua",
                "planted_gate",
                "state-equality",
                "merged",
                12,
            )
        }
        allowlist = {
            monotone.Violation(
                "packages/github-devloop/core/synthetic_gate.lua",
                "planted_gate",
                "state-equality",
                "pr-open",
                12,
            )
        }

        messages = monotone.ratchet_messages(current, allowlist, allowlist)

        joined = "\n".join(messages)
        self.assertIn("state-equality merged", joined)
        self.assertIn("unclassified transient lifecycle cursor read", joined)

    def test_v2_moving_read_to_different_file_is_still_growth(self) -> None:
        current = {
            monotone.Violation(
                "packages/github-devloop/core/moved_gate.lua",
                "planted_gate",
                "cursor-read",
                "current_state(",
                12,
            )
        }
        allowlist = {
            monotone.Violation(
                "packages/github-devloop/core/synthetic_gate.lua",
                "planted_gate",
                "cursor-read",
                "current_state(",
                12,
            )
        }

        messages = monotone.ratchet_messages(current, allowlist, allowlist)

        joined = "\n".join(messages)
        self.assertIn("packages/github-devloop/core/moved_gate.lua:12", joined)
        self.assertIn("unclassified transient lifecycle cursor read", joined)

    def test_v2_same_bucket_relocation_within_function_passes(self) -> None:
        # Accepted blind spot (by design, confirmed via cross-model review): a
        # count-preserving relocation of an existing raw read WITHIN the same
        # (path, enclosing function, kind, canonical token) bucket is not growth.
        # The gate prevents increases in semantic raw-read debt; it does not
        # police intra-function control-flow relocation. Pinned so a future
        # reviewer does not rediscover this and treat it as a bug.
        key = (
            "packages/github-devloop/core/synthetic_gate.lua",
            "planted_gate",
            "cursor-read",
            "current_state(",
        )
        # allowlist: one read at line 10; current: the same-bucket read relocated
        # to line 80 (different branch / line) -- count 1 -> 1.
        allowlist = [monotone.Violation(*key, 10)]
        current = [monotone.Violation(*key, 80)]

        messages = monotone.ratchet_messages(current, allowlist, allowlist)

        joined = "\n".join(messages)
        self.assertNotIn("grows monotone-gate", joined)
        self.assertNotIn("shrank", joined)
        self.assertEqual(messages, [])


if __name__ == "__main__":
    unittest.main()
