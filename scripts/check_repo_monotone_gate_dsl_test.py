#!/usr/bin/env python3
"""Unit tests for the G-MONOTONE-GATE-DSL repository guard."""

from __future__ import annotations

import importlib.util
import sys
import tempfile
import textwrap
import unittest
from pathlib import Path


def load_module():
    path = Path(__file__).with_name("check_repo_monotone_gate_dsl.py")
    spec = importlib.util.spec_from_file_location("check_repo_monotone_gate_dsl", path)
    if spec is None or spec.loader is None:
        raise RuntimeError("could not load check_repo_monotone_gate_dsl.py")
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


dsl = load_module()


def write_gate(root: Path, name: str, source: str) -> None:
    target = root / "packages" / "github-devloop" / "core" / "gates" / name
    target.parent.mkdir(parents=True)
    target.write_text("return [=[\n" + textwrap.dedent(source) + "]=]\n", encoding="utf-8")
    (root / "migration").mkdir(exist_ok=True)
    (root / dsl.ALLOWLIST).write_text("", encoding="utf-8")


def write_raw_gate(root: Path, name: str, source: str) -> None:
    target = root / "packages" / "github-devloop" / "core" / "gates" / name
    target.parent.mkdir(parents=True)
    target.write_text(textwrap.dedent(source), encoding="utf-8")
    (root / "migration").mkdir(exist_ok=True)
    (root / dsl.ALLOWLIST).write_text("", encoding="utf-8")


class MonotoneGateDslRatchetTest(unittest.TestCase):
    def test_gate_definition_may_require_only_gate_dsl(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            write_gate(
                root,
                "bad.lua",
                """\
                local state = require("devloop.state")

                return require_reached("pr-open", {
                  domain = "github-devloop-pr",
                  raw = state.current_state,
                })
                """,
            )

            messages = dsl.repository_messages(root, enforce_base=False)

        joined = "\n".join(messages)
        self.assertIn("require devloop.state", joined)
        self.assertIn("dangerous-global require", joined)
        self.assertIn("raw-token current_state", joined)
        self.assertIn("forbidden in a core/gates DSL definition", joined)

    def test_gate_definition_rejects_require_alias_as_backstop(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            write_gate(
                root,
                "bad_alias_require.lua",
                """\
                local r = require
                r("debug")

                return require_reached("pr-open", {
                  domain = "github-devloop-pr",
                })
                """,
            )

            messages = dsl.repository_messages(root, enforce_base=False)

        joined = "\n".join(messages)
        self.assertIn("dangerous-global require", joined)
        self.assertIn("forbidden in a core/gates DSL definition", joined)

    def test_gate_definition_rejects_debug_reflection_smuggle(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            write_gate(
                root,
                "bad_debug.lua",
                """\
                local raw = debug.getupvalue(require_reached, 1)

                return require_reached("pr-open", {
                  domain = "github-devloop-pr",
                  raw = raw,
                })
                """,
            )

            messages = dsl.repository_messages(root, enforce_base=False)

        joined = "\n".join(messages)
        self.assertIn("dangerous-global debug", joined)
        self.assertIn("forbidden in a core/gates DSL definition", joined)

    def test_gate_definition_rejects_monkey_patch_smuggle(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            write_gate(
                root,
                "bad_patch.lua",
                """\
                local gate = require("devloop.gate")
                gate.holds = function()
                  return true
                end

                return gate.require_reached("pr-open", {
                  domain = "github-devloop-pr",
                })
                """,
            )

            messages = dsl.repository_messages(root, enforce_base=False)

        joined = "\n".join(messages)
        self.assertIn("dangerous-global require", joined)
        self.assertIn("monkey-patch gate", joined)
        self.assertIn("forbidden in a core/gates DSL definition", joined)

    def test_gate_definition_rejects_direct_require_monkey_patch(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            write_gate(
                root,
                "bad_direct_patch.lua",
                """\
                require("devloop.gate").holds = function()
                  return true
                end
                local gate = require("devloop.gate")

                return gate.require_reached("pr-open", {
                  domain = "github-devloop-pr",
                })
                """,
            )

            messages = dsl.repository_messages(root, enforce_base=False)

        joined = "\n".join(messages)
        self.assertIn("dangerous-global require", joined)
        self.assertIn("monkey-patch devloop.gate", joined)
        self.assertIn("forbidden in a core/gates DSL definition", joined)

    def test_production_code_must_not_require_gate_defs_directly(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            write_gate(
                root,
                "child_start_visible.lua",
                """\
                return require_reached("pr-open", {
                  domain = "github-devloop-pr",
                })
                """,
            )
            bypass = root / "packages" / "github-devloop" / "core" / "bypass.lua"
            bypass.parent.mkdir(parents=True, exist_ok=True)
            bypass.write_text(
                textwrap.dedent(
                    """\
                    local gate_def = require("core.gates.child_start_visible")
                    return gate_def
                    """
                ),
                encoding="utf-8",
            )

            messages = dsl.repository_messages(root, enforce_base=False)

        joined = "\n".join(messages)
        self.assertIn("loader-bypass core.gates.child_start_visible", joined)
        self.assertIn("restricted_lua_load sandbox is authoritative", joined)

    def test_package_wiring_may_resolve_gate_source_module(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            write_gate(
                root,
                "child_start_visible.lua",
                """\
                return require_reached("pr-open", {
                  domain = "github-devloop-pr",
                })
                """,
            )
            wiring = root / "packages" / "github-devloop" / "core" / "devloop_wiring.lua"
            wiring.parent.mkdir(parents=True, exist_ok=True)
            wiring.write_text(
                textwrap.dedent(
                    """\
                    local W = {}
                    function W.gate_sources()
                      return {
                        child_start_visible = require("core.gates.child_start_visible"),
                      }
                    end
                    return W
                    """
                ),
                encoding="utf-8",
            )

            messages = dsl.repository_messages(root, enforce_base=False)

        self.assertEqual(messages, [])

    def test_gate_file_must_return_source_string(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            write_raw_gate(
                root,
                "raw.lua",
                """\
                return require_reached("pr-open", {
                  domain = "github-devloop-pr",
                })
                """,
            )

            messages = dsl.repository_messages(root, enforce_base=False)

        joined = "\n".join(messages)
        self.assertIn("core/gates/raw.lua must return a literal DSL source string", joined)

    def test_any_lua_code_must_not_require_gate_defs_directly(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            write_gate(
                root,
                "child_start_visible.lua",
                """\
                return require_reached("pr-open", {
                  domain = "github-devloop-pr",
                })
                """,
            )
            bypass = root / "packages" / "github-devloop" / "tests" / "bypass_test.lua"
            bypass.parent.mkdir(parents=True, exist_ok=True)
            bypass.write_text(
                textwrap.dedent(
                    """\
                    local gate_def = require("core.gates.child_start_visible")
                    return gate_def
                    """
                ),
                encoding="utf-8",
            )

            messages = dsl.repository_messages(root, enforce_base=False)

        joined = "\n".join(messages)
        self.assertIn("packages/github-devloop/tests/bypass_test.lua:1 loader-bypass core.gates.child_start_visible", joined)
        self.assertIn("restricted_lua_load sandbox is authoritative", joined)

    def test_any_lua_code_must_not_path_load_gate_defs_directly(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            write_gate(
                root,
                "child_start_visible.lua",
                """\
                return require_reached("pr-open", {
                  domain = "github-devloop-pr",
                })
                """,
            )
            bypass = root / "packages" / "github-devloop" / "core" / "path_bypass.lua"
            bypass.parent.mkdir(parents=True, exist_ok=True)
            bypass.write_text(
                textwrap.dedent(
                    """\
                    local path = package_root .. "/core/gates/child_start_visible.lua"
                    return dofile(path)
                    """
                ),
                encoding="utf-8",
            )

            messages = dsl.repository_messages(root, enforce_base=False)

        joined = "\n".join(messages)
        self.assertIn("loader-bypass /core/gates/child_start_visible.lua", joined)
        self.assertIn("restricted_lua_load sandbox is authoritative", joined)

    def test_any_lua_code_must_not_split_literal_path_load_gate_defs(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            write_gate(
                root,
                "child_start_visible.lua",
                """\
                return require_reached("pr-open", {
                  domain = "github-devloop-pr",
                })
                """,
            )
            bypass = root / "packages" / "github-devloop" / "core" / "split_path_bypass.lua"
            bypass.parent.mkdir(parents=True, exist_ok=True)
            bypass.write_text(
                textwrap.dedent(
                    """\
                    local path = package_root .. "/core/" .. "gates/" .. "child_start_visible.lua"
                    return dofile(path)
                    """
                ),
                encoding="utf-8",
            )

            messages = dsl.repository_messages(root, enforce_base=False)

        joined = "\n".join(messages)
        self.assertIn("loader-bypass /core/gates/child_start_visible.lua", joined)
        self.assertIn("restricted_lua_load sandbox is authoritative", joined)

    def test_pure_gate_definition_passes(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            write_gate(
                root,
                "good.lua",
                """\
                return require_reached("pr-open", {
                  domain = "github-devloop-pr",
                  lineage = {
                    proposal_id = true,
                  },
                })
                """,
            )

            messages = dsl.repository_messages(root, enforce_base=False)

        self.assertEqual(messages, [])


if __name__ == "__main__":
    unittest.main()
