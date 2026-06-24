#!/usr/bin/env python3
"""Tests for the shell-out-to-self migration ratchet."""

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
check_repo = load_module("check_repo", scripts_dir / "check_repo.py")
shell_out = check_repo.check_repo_shell_out_to_self


class ShellOutToSelfRatchetTest(unittest.TestCase):
    def sites(self, source: str) -> set[str]:
        return shell_out.source_sites(
            "packages/example/core.lua",
            source,
            check_repo.strip_lua_comments_and_strings,
            check_repo.lua_string_literals,
        )

    def test_detects_inline_exec_argv_to_engine_binary(self) -> None:
        source = """
local result = exec_argv({ argv = { BIN, "doctor" }, timeout = 30 })
"""
        sites = self.sites(source)

        self.assertEqual(sites, {"packages/example/core.lua:line=2:argv:engine-binary"})

    def test_detects_pcall_wrapped_run_argv_to_engine_binary(self) -> None:
        source = """
local ok, res = pcall(run_argv, { argv = { bin, "observe", "--json" }, timeout = 30 })
"""
        sites = self.sites(source)

        self.assertEqual(sites, {"packages/example/core.lua:line=2:argv:engine-binary"})

    def test_detects_pcall_wrapped_positional_run_argv_to_engine_binary(self) -> None:
        source = """
local ok, res = pcall(run_argv, { bin, "observe", "--json" })
"""
        sites = self.sites(source)

        self.assertEqual(sites, {"packages/example/core.lua:line=2:argv:engine-binary"})

    def test_detects_xpcall_wrapped_exec_alias_to_engine_binary(self) -> None:
        source = """
local run = exec.exec_argv
local ok, res = xpcall(run, debug.traceback, { argv = { BIN, "observe", "--json" } })
"""
        sites = self.sites(source)

        self.assertEqual(sites, {"packages/example/core.lua:line=3:argv:engine-binary"})

    def test_detects_executor_alias_call_to_engine_binary(self) -> None:
        source = """
local sh = exec_argv
sh({ argv = { BIN, "observe" }, timeout = 30 })
"""
        sites = self.sites(source)

        self.assertEqual(sites, {"packages/example/core.lua:line=3:argv:engine-binary"})

    def test_detects_positional_executor_alias_call_to_engine_binary(self) -> None:
        source = """
local sh = exec_argv
sh({ BIN, "observe" })
"""
        sites = self.sites(source)

        self.assertEqual(sites, {"packages/example/core.lua:line=3:argv:engine-binary"})

    def test_detects_split_constructed_argv_to_engine_binary(self) -> None:
        source = """
local framework_bin = os.getenv("BIN")
local argv = { framework_bin, "observe", "--json" }
local result = exec_argv({ argv = argv, timeout = 30 })
"""
        sites = self.sites(source)

        self.assertEqual(sites, {"packages/example/core.lua:line=4:argv:engine-binary"})

    def test_detects_split_exec_alias_to_engine_binary(self) -> None:
        source = """
local local_bin = os.getenv("BIN")
local argv = { local_bin, "observe", "--json" }
local sh = exec_argv
local result = sh({ argv = argv, timeout = 30 })
"""
        sites = self.sites(source)

        self.assertEqual(sites, {"packages/example/core.lua:line=5:argv:engine-binary"})

    def test_detects_run_argv_literal_engine_binary(self) -> None:
        source = """
local result = run_argv({
  argv = { "fkst-framework", "test", "--package-root", root },
})
"""
        sites = self.sites(source)

        self.assertEqual(sites, {"packages/example/core.lua:line=2:argv:engine-binary"})

    def test_detects_positional_exec_argv_to_engine_binary(self) -> None:
        source = """
local result = exec_argv({ BIN, "observe" })
"""
        sites = self.sites(source)

        self.assertEqual(sites, {"packages/example/core.lua:line=2:argv:engine-binary"})

    def test_detects_sync_string_shell_out_to_engine_binary(self) -> None:
        source = """
local result = exec_sync({ cmd = "$BIN observe --json", timeout = 30 })
"""
        sites = self.sites(source)

        self.assertEqual(sites, {"packages/example/core.lua:line=2:sync:engine-binary"})

    def test_detects_positional_sync_string_shell_out_to_engine_binary(self) -> None:
        source = """
local result = exec_sync("$BIN observe --json")
"""
        sites = self.sites(source)

        self.assertEqual(sites, {"packages/example/core.lua:line=2:sync:engine-binary"})

    def test_detects_sync_executor_alias_call_to_engine_binary(self) -> None:
        source = """
local sh = exec_sync
sh("$BIN observe")
"""
        sites = self.sites(source)

        self.assertEqual(sites, {"packages/example/core.lua:line=3:sync:engine-binary"})

    def test_detects_sync_engine_alias(self) -> None:
        source = """
local framework_bin = os.getenv("BIN")
local cmd = framework_bin .. " health"
local result = run_sync({ cmd = cmd, timeout = 30 })
"""
        sites = self.sites(source)

        self.assertEqual(sites, {"packages/example/core.lua:line=4:sync:engine-binary"})

    def test_detects_sync_string_shell_out_when_engine_binary_is_not_first_token(self) -> None:
        source = """
local result = exec_sync({ cmd = "cd /tmp && $BIN observe --json", timeout = 30 })
"""
        sites = self.sites(source)

        self.assertEqual(sites, {"packages/example/core.lua:line=2:sync:engine-binary"})

    def test_detects_exec_argv_table_alias_head_from_bin_alias(self) -> None:
        source = """
local local_bin = BIN
local argv = { local_bin, "--self-test" }
exec_argv({ timeout = 30, argv = argv })
"""
        sites = self.sites(source)

        self.assertEqual(sites, {"packages/example/core.lua:line=4:argv:engine-binary"})

    def test_detects_simple_variable_opts_passed_to_exec_alias(self) -> None:
        source = """
local opts = { argv = { BIN, "observe" } }
local sh = exec_argv
sh(opts)
"""
        sites = self.sites(source)

        self.assertEqual(sites, {"packages/example/core.lua:line=4:argv:engine-binary"})

    def test_uses_top_level_argv_when_nested_metadata_comes_first(self) -> None:
        source = """
exec_argv({
  metadata = { argv = { "git", "status" } },
  argv = { BIN, "observe", "--json" },
})
"""
        sites = self.sites(source)

        self.assertEqual(sites, {"packages/example/core.lua:line=2:argv:engine-binary"})

    def test_ignores_nested_metadata_argv_when_top_level_argv_is_benign(self) -> None:
        source = """
exec_argv({
  metadata = { argv = { BIN, "observe", "--json" } },
  argv = { "git", "status" },
})
"""
        sites = self.sites(source)

        self.assertEqual(sites, set())

    def test_ignores_comments(self) -> None:
        source = """
-- exec_argv({ argv = { bin, "observe", "--json" } })
"""
        self.assertEqual(self.sites(source), set())

    def test_current_tree_has_zero_violations(self) -> None:
        root = scripts_dir.parent
        sites = shell_out.sites(
            root,
            check_repo.package_roots(root),
            check_repo.read_text,
            check_repo.rel,
            check_repo.strip_lua_comments_and_strings,
            check_repo.lua_string_literals,
        )

        self.assertEqual(sites, set())

    def test_repository_ratchet_catches_inline_and_split_fixture(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            package = root / "packages" / "example"
            package.mkdir(parents=True)
            (package / "fkst.toml").write_text('name = "example"\n', encoding="utf-8")
            (package / "core.lua").write_text(
                """
local function inline()
  return exec_argv({ argv = { BIN, "doctor" }, timeout = 30 })
end

local function wrapped()
  local ok, res = pcall(run_argv, { argv = { BIN, "observe", "--json" } })
  return ok, res
end

local function wrapped_positional()
  local ok, res = pcall(run_argv, { BIN, "observe", "--json" })
  return ok, res
end

local function alias()
  local sh = exec_argv
  return sh({ argv = { BIN, "observe" }, timeout = 30 })
end

local function alias_positional()
  local sh = exec_argv
  return sh({ BIN, "observe" })
end

local function alias_sync()
  local sh = exec_sync
  return sh("$BIN observe")
end

local function split()
  local framework_bin = os.getenv("BIN")
  local argv = { framework_bin, "observe", "--json" }
  return exec_argv({ argv = argv, timeout = 30 })
end

local function nested()
  return exec_argv({
    metadata = { argv = { "git", "status" } },
    argv = { BIN, "observe", "--json" },
  })
end
""",
                encoding="utf-8",
            )

            violations: list[str] = []
            check_repo.check_shell_out_to_self_ratchet(root, violations)

        self.assertEqual(len(violations), 8)
        self.assertTrue(all("G-SHELL-OUT-TO-SELF" in violation for violation in violations))
        self.assertTrue(any("line=3:argv:engine-binary" in violation for violation in violations))
        self.assertTrue(any("line=7:argv:engine-binary" in violation for violation in violations))
        self.assertTrue(any("line=12:argv:engine-binary" in violation for violation in violations))
        self.assertTrue(any("line=18:argv:engine-binary" in violation for violation in violations))
        self.assertTrue(any("line=23:argv:engine-binary" in violation for violation in violations))
        self.assertTrue(any("line=28:sync:engine-binary" in violation for violation in violations))
        self.assertTrue(any("line=34:argv:engine-binary" in violation for violation in violations))
        self.assertTrue(any("line=38:argv:engine-binary" in violation for violation in violations))

    def test_allowlist_and_stale_entries(self) -> None:
        site = "packages/example/core.lua:line=2:argv:engine-binary"
        current = {site}

        self.assertEqual(shell_out.ratchet_messages(current, {site}), [])
        messages = shell_out.ratchet_messages(set(), {site})
        self.assertEqual(len(messages), 1)
        self.assertIn("no longer detected", messages[0])


if __name__ == "__main__":
    unittest.main()
