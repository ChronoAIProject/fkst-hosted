#!/usr/bin/env python3
"""Tests for the library require cycle ratchet."""

from __future__ import annotations

import importlib.util
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock


def load_module(name: str, path: Path):
    spec = importlib.util.spec_from_file_location(name, path)
    if spec is None or spec.loader is None:
        raise RuntimeError(f"could not load {path}")
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


scripts_dir = Path(__file__).resolve().parent
check_repo = load_module("check_repo", scripts_dir / "check_repo.py")
dependency_cycle = load_module("check_repo_dependency_cycle", scripts_dir / "check_repo_dependency_cycle.py")


def write(path: Path, source: str) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(source, encoding="utf-8")


class DependencyCycleGuardTest(unittest.TestCase):
    def current_cycles(self, root: Path) -> set[str]:
        return dependency_cycle.current_cycles(
            root,
            check_repo.read_text,
            check_repo.strip_lua_comments_and_strings,
            check_repo.is_unmasked_range,
        )

    def run_messages(self, root: Path, allowlist: str = "") -> list[str]:
        write(root / dependency_cycle.ALLOWLIST, allowlist)
        with mock.patch.object(dependency_cycle, "allowlist_at_dev_base", return_value=("absent", None)):
            return dependency_cycle.messages(
                root,
                check_repo.read_text,
                check_repo.strip_lua_comments_and_strings,
                check_repo.is_unmasked_range,
                enforce_base=True,
            )

    def test_detects_two_node_cycle(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            write(root / "libraries" / "demo" / "a.lua", 'require("demo.b")\n')
            write(root / "libraries" / "demo" / "b.lua", 'require("demo.a")\n')

            cycles = self.current_cycles(root)

        self.assertEqual(cycles, {"demo.a <-> demo.b"})

    def test_detects_three_node_cycle(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            write(root / "libraries" / "demo" / "a.lua", 'require("demo.b")\n')
            write(root / "libraries" / "demo" / "b.lua", 'require("demo.c")\n')
            write(root / "libraries" / "demo" / "c.lua", 'require("demo.a")\n')

            cycles = self.current_cycles(root)

        self.assertEqual(cycles, {"demo.a <-> demo.b <-> demo.c"})

    def test_acyclic_graph_is_not_flagged(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            write(root / "libraries" / "demo" / "a.lua", 'require("demo.b")\n')
            write(root / "libraries" / "demo" / "b.lua", 'require("demo.c")\n')
            write(root / "libraries" / "demo" / "c.lua", "return {}\n")

            cycles = self.current_cycles(root)
            messages = self.run_messages(root)

        self.assertEqual(cycles, set())
        self.assertEqual(messages, [])

    def test_real_repo_has_only_allowlisted_cycles(self) -> None:
        root = Path(__file__).resolve().parents[1]
        with mock.patch.object(dependency_cycle, "allowlist_at_dev_base", return_value=("absent", None)):
            messages = dependency_cycle.messages(
                root,
                check_repo.read_text,
                check_repo.strip_lua_comments_and_strings,
                check_repo.is_unmasked_range,
                enforce_base=True,
            )

        self.assertEqual(messages, [])
        self.assertEqual(
            self.current_cycles(root),
            {
                "devloop.autonomy_ledger <-> devloop.markers.builders <-> devloop.state",
                "devloop.claims <-> devloop.entity",
                "devloop.claims <-> devloop.entity <-> devloop.requests.labels <-> devloop.state",
                "devloop.claims <-> devloop.forks",
                "devloop.claims <-> devloop.forks <-> devloop.parsers.issue",
                "devloop.claims <-> devloop.requests.labels <-> devloop.state",
                "devloop.payloads.predicates <-> devloop.state",
                "devloop.requests.labels <-> devloop.state",
            },
        )

    def test_new_cycle_not_in_allowlist_fails(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            write(root / "libraries" / "demo" / "a.lua", 'require("demo.b")\n')
            write(root / "libraries" / "demo" / "b.lua", 'require("demo.a")\n')

            messages = self.run_messages(root, allowlist="")

        self.assertEqual(len(messages), 1)
        self.assertIn("demo.a <-> demo.b", messages[0])
        self.assertIn("not in migration/dependency-cycle.allowlist", messages[0])


if __name__ == "__main__":
    unittest.main()
