#!/usr/bin/env python3
"""Tests for the transition-version suffix ratchet."""

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
version_suffix = load_module("check_repo_version_suffix", scripts_dir / "check_repo_version_suffix.py")


class VersionSuffixRatchetTest(unittest.TestCase):
    def test_detects_raw_suffix_construction_in_concat_context(self) -> None:
        source = """
local function next_version(base, n)
  return base .. "/loop/" .. tostring(n)
end
local marker = "/loop/"
"""
        sites = version_suffix.source_sites("packages/example/core.lua", source)

        self.assertEqual(len(sites), 1)
        site = next(iter(sites))
        self.assertEqual(site.path, "packages/example/core.lua")
        self.assertEqual(site.line, 3)
        self.assertEqual(site.kind, "construction")
        self.assertIn('"/loop/"', site.text)

    def test_detects_ad_hoc_suffix_parsing_patterns(self) -> None:
        source = """
local function strip(value)
  return tostring(value):gsub("/timeout/[%w%-]+/%d+$", "")
end
local function round(value)
  return tostring(value):match("^(.-)/review%-loop/%d+")
end
"""
        sites = version_suffix.source_sites("libraries/devloop/example.lua", source)

        self.assertEqual(
            {(site.line, site.kind) for site in sites},
            {(3, "parsing"), (6, "parsing")},
        )

    def test_parsing_detection_requires_literal_inside_call_arguments(self) -> None:
        source = """
local function key(timestamp, suffix)
  return timestamp:gsub(":", "-") .. "/loop/" .. suffix
end
"""
        sites = version_suffix.source_sites("libraries/contract/source_ref.lua", source)

        self.assertEqual(
            {(site.line, site.kind) for site in sites},
            {(3, "construction")},
        )

    def test_ignores_tests_and_sanctioned_transition_module(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "libraries/contract").mkdir(parents=True)
            (root / "libraries/contract/transition_version.lua").write_text(
                'return base .. "/loop/" .. tostring(n)\n',
                encoding="utf-8",
            )
            (root / "packages/example/tests").mkdir(parents=True)
            (root / "packages/example/tests/example_test.lua").write_text(
                'return base .. "/loop/" .. tostring(n)\n',
                encoding="utf-8",
            )
            (root / "packages/example/core.lua").write_text(
                'return base .. "/fix/" .. tostring(n)\n',
                encoding="utf-8",
            )

            current = version_suffix.sites(root)

        self.assertEqual({site.path for site in current}, {"packages/example/core.lua"})

    def test_allowlisted_site_passes_and_stale_entry_fails(self) -> None:
        site = version_suffix.VersionSuffixSite(
            "packages/example/core.lua",
            2,
            "construction",
            'return base .. "/loop/" .. tostring(n)',
        )
        allow = {
            version_suffix.VersionSuffixAllowlistEntry.parse(
                'packages/example/core.lua:2 # why=legacy dedup-key suffix construction'
            )
        }

        self.assertEqual(version_suffix.ratchet_messages({site}, allow), [])
        messages = version_suffix.ratchet_messages(set(), allow)
        self.assertEqual(len(messages), 1)
        self.assertIn("prune the stale entry", messages[0])


if __name__ == "__main__":
    unittest.main()
