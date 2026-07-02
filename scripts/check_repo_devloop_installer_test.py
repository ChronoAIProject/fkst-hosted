#!/usr/bin/env python3
"""Tests for the G-DEVLOOP-INSTALLER shrink-only ratchet (install(M) composed-core coupling)."""
from __future__ import annotations

import importlib.util
import sys
import tempfile
import unittest
from pathlib import Path


def load_module(name: str, path: Path):
    spec = importlib.util.spec_from_file_location(name, path)
    if spec is None or spec.loader is None:
        raise RuntimeError(f"could not load {path}")
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


scripts_dir = Path(__file__).resolve().parent
ratchet = load_module("check_repo_devloop_installer", scripts_dir / "check_repo_devloop_installer.py")


def scaffold(root: Path, *, mod_files: dict[str, str], core: str, readers: dict[str, str]) -> None:
    (root / "libraries" / "devloop").mkdir(parents=True, exist_ok=True)
    for rel, body in mod_files.items():
        p = root / "libraries" / rel
        p.parent.mkdir(parents=True, exist_ok=True)
        p.write_text(body, encoding="utf-8")
    pkg = root / "packages" / "p"
    (pkg / "departments" / "d").mkdir(parents=True, exist_ok=True)
    (pkg / "core.lua").write_text(core, encoding="utf-8")
    for fname, body in readers.items():
        (pkg / "departments" / "d" / fname).write_text(body, encoding="utf-8")


class InstallerRatchetTest(unittest.TestCase):
    def _count(self, *, mod_files, core, readers) -> int:
        with tempfile.TemporaryDirectory() as td:
            root = Path(td)
            scaffold(root, mod_files=mod_files, core=core, readers=readers)
            return ratchet.reader_calls(root, ratchet.installer_symbols(root))

    def test_installed_symbol_read_counts(self):
        n = self._count(
            mod_files={"devloop/logging.lua": "function S.install(M)\nfunction M.log_raise(a) end\nend\nreturn S\n"},
            core='require("devloop.logging").install(M)\n',
            readers={"main.lua": "core.log_raise(x)\n"},
        )
        self.assertEqual(n, 1)

    def test_assignment_style_installer_symbol_counts(self):
        # methods installed via `M.name = ...` (not `function M.name`) must also count
        n = self._count(
            mod_files={"devloop/logging.lua": "function S.install(M)\nM.payload_field = other.payload_field\nend\nreturn S\n"},
            core='require("devloop.logging").install(M)\n',
            readers={"main.lua": "core.payload_field(x)\n"},
        )
        self.assertEqual(n, 1)

    def test_non_installed_symbol_does_not_count(self):
        n = self._count(
            mod_files={"devloop/logging.lua": "function S.install(M)\nfunction M.log_raise(a) end\nend\nreturn S\n"},
            core='require("devloop.logging").install(M)\n',
            readers={"main.lua": "core.some_other_fn(x)\n"},
        )
        self.assertEqual(n, 0)

    def test_module_not_installed_does_not_count(self):
        # logging defines log_raise but the core never install()s it -> not an installer symbol
        n = self._count(
            mod_files={"devloop/logging.lua": "function S.install(M)\nfunction M.log_raise(a) end\nend\nreturn S\n"},
            core='local x = 1\n',
            readers={"main.lua": "core.log_raise(x)\n"},
        )
        self.assertEqual(n, 0)

    def test_aggregator_submodule_symbols_count(self):
        # commands aggregator loops submodules; a submodule's installed method must count
        n = self._count(
            mod_files={
                "devloop/commands.lua": 'local modules = {"devloop.commands.prs"}\nfunction S.install(M)\nfor _,m in ipairs(modules) do require(m).install(M) end\nend\nreturn S\n',
                "devloop/commands/prs.lua": "function S.install(M)\nfunction M.gh_pr_view_observe(a) end\nend\nreturn S\n",
            },
            core='require("devloop.commands").install(M)\n',
            readers={"main.lua": "core.gh_pr_view_observe(x)\n"},
        )
        self.assertEqual(n, 1)

    def test_core_and_test_readers_excluded(self):
        with tempfile.TemporaryDirectory() as td:
            root = Path(td)
            scaffold(
                root,
                mod_files={"devloop/logging.lua": "function S.install(M)\nfunction M.log_raise(a) end\nend\nreturn S\n"},
                core='require("devloop.logging").install(M)\ncore.log_raise(inside_core)\n',
                readers={},
            )
            (root / "packages" / "p" / "tests").mkdir(parents=True, exist_ok=True)
            (root / "packages" / "p" / "tests" / "t_test.lua").write_text("core.log_raise(x)\n", encoding="utf-8")
            self.assertEqual(ratchet.reader_calls(root, ratchet.installer_symbols(root)), 0)


if __name__ == "__main__":
    unittest.main()
