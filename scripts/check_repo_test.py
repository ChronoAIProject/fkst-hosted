#!/usr/bin/env python3
"""Unit tests for repository guard helpers."""

from __future__ import annotations

import importlib.util
import sys
import unittest
from pathlib import Path


def load_check_repo():
    path = Path(__file__).with_name("check_repo.py")
    spec = importlib.util.spec_from_file_location("check_repo", path)
    if spec is None or spec.loader is None:
        raise RuntimeError("could not load check_repo.py")
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


check_repo = load_check_repo()


class GraphqlConnectionGuardTest(unittest.TestCase):
    def warning_lines(self, source: str) -> list[int]:
        return check_repo.unguarded_graphql_first_connection_lines(source)

    def test_warns_first_connection_without_guard(self) -> None:
        source = """
local query = [[
  query { repository(owner: "o", name: "r") { issues(first:10) { nodes { number } } } }
]]
"""
        self.assertEqual(self.warning_lines(source), [3])

    def test_allows_total_count_guard(self) -> None:
        source = """
local query = 'query { repository(owner:"o", name:"r") { issues(first:10) { totalCount nodes { number } } } }'
"""
        self.assertEqual(self.warning_lines(source), [])

    def test_allows_page_info_has_next_page_guard(self) -> None:
        source = """
local query = 'query { repository(owner:"o", name:"r") { issues(first:10) { pageInfo { hasNextPage } nodes { number } } } }'
"""
        self.assertEqual(self.warning_lines(source), [])

    def test_warns_page_info_without_has_next_page(self) -> None:
        source = """
local query = 'query { repository(owner:"o", name:"r") { issues(first:10) { pageInfo { endCursor } nodes { number } } } }'
"""
        self.assertEqual(self.warning_lines(source), [2])

    def test_ignores_comments(self) -> None:
        source = """
-- query { repository(owner:"o", name:"r") { issues(first:10) { nodes { number } } } }
local query = 'query { repository(owner:"o", name:"r") { issues(first:10) { totalCount nodes { number } } } }'
"""
        self.assertEqual(self.warning_lines(source), [])


class RestPaginationGuardTest(unittest.TestCase):
    def warning_lines(self, source: str) -> list[int]:
        return check_repo.unguarded_rest_per_page_lines(source)

    def test_warns_fixed_rest_page_without_paginate(self) -> None:
        source = """
local cmd = "gh api 'repos/o/r/issues?state=open&per_page=100'"
"""
        self.assertEqual(self.warning_lines(source), [2])

    def test_allows_paginated_rest_read(self) -> None:
        source = """
local cmd = "gh api --paginate --slurp "
  .. shell_quote("repos/o/r/issues?state=open&per_page=100")
"""
        self.assertEqual(self.warning_lines(source), [])


class HiddenTextGuardTest(unittest.TestCase):
    def hidden_lines(self, source: str) -> list[int]:
        return check_repo.hidden_text_encoded_literal_lines(source)

    def test_warns_decode_helper_wrapped_hex_literal(self) -> None:
        source = """
local function h(value) return value end
local label = h("6769746875622d6465766c6f6f7020e6809de88083")
"""
        self.assertEqual(self.hidden_lines(source), [3])

    def test_warns_decode_helper_wrapped_base64_literal(self) -> None:
        source = """
local label = base64_decode("Z2l0aHViLWRldmxvb3AgdGhpbmtpbmc=")
"""
        self.assertEqual(self.hidden_lines(source), [2])

    def test_warns_decode_helper_wrapped_byte_escape_literal(self) -> None:
        source = r'''
local label = decode_bytes("\xe4\xb8\x89\xe8\xa7\x92")
'''
        self.assertEqual(self.hidden_lines(source), [2])

    def test_warns_long_string_char_byte_sequence(self) -> None:
        source = """
local label = string.char(0xe4, 0xb8, 0x89, 0xe8, 0xa7, 0x92)
"""
        self.assertEqual(self.hidden_lines(source), [2])

    def test_ignores_comments_and_plain_literals(self) -> None:
        source = """
-- local label = h("6769746875622d6465766c6f6f7020e6809de88083")
local digest = "6769746875622d6465766c6f6f7020e6809de88083"
local token = encode_hex("plain text")
local encoded = encode_hex("6769746875622d6465766c6f6f7020e6809de88083")
"""
        self.assertEqual(self.hidden_lines(source), [])

    def test_github_devloop_zh_strings_are_source_greppable(self) -> None:
        root = Path(__file__).resolve().parents[1]
        probe = bytes.fromhex("e4b889e8a792e585b1e8af86e69caae8bebee68890").decode("utf-8")
        hits = [
            path
            for path in root.rglob("*.lua")
            if probe in path.read_text(encoding="utf-8")
        ]
        self.assertIn(root / "packages/github-devloop/core/strings.lua", hits)


class GhRatePoolSizingGuardTest(unittest.TestCase):
    def sizing_lines(self, source: str) -> list[int]:
        return check_repo.gh_rate_pool_sizing_lines(source)

    def test_warns_on_hardcoded_gh_pool_sizing(self) -> None:
        source = """
function M.gh_rate_pool()
  return { name = "gh", burst = 50, refill_per_hour = 3250 }
end
"""
        self.assertEqual(self.sizing_lines(source), [3])

    def test_allows_name_only_pool_and_unrelated_sizing_fields(self) -> None:
        source = """
function M.gh_rate_pool()
  return { name = "gh" }
end

local unrelated = { burst = 50, refill_per_hour = 3250 }
"""
        self.assertEqual(self.sizing_lines(source), [])

    def test_ignores_comments_and_strings(self) -> None:
        source = """
function M.gh_rate_pool()
  -- burst = 50
  return { name = "gh", note = "refill_per_hour" }
end
"""
        self.assertEqual(self.sizing_lines(source), [])


class RunScriptContractTest(unittest.TestCase):
    def source(self) -> str:
        return Path(__file__).with_name("run.sh").read_text(encoding="utf-8")

    def test_supervise_requires_shared_rate_pool_root(self) -> None:
        source = self.source()

        self.assertIn('if [ -z "${FKST_RATE_POOL_ROOT:-}" ]; then', source)
        self.assertIn("FKST_RATE_POOL_ROOT is required for supervise", source)
        self.assertIn("FKST_RATE_POOL_ROOT must be an absolute host-stable directory path", source)
        self.assertIn('echo "FKST_RATE_POOL_ROOT=$FKST_RATE_POOL_ROOT"', source)

    def test_python_repository_checks_do_not_write_bytecode_cache(self) -> None:
        source = self.source()

        self.assertIn('python3 -B "$ROOT/scripts/check_repo.py"', source)
        self.assertIn('python3 -B "$ROOT/scripts/check_repo_test.py"', source)
        self.assertIn('python3 -B "$ROOT/scripts/bin_cache_test.py"', source)
        self.assertIn('python3 -B "$ROOT/scripts/bin_bootstrap_test.py"', source)
        self.assertNotIn('python3 "$ROOT/scripts/check_repo.py"', source)
        self.assertNotIn('python3 "$ROOT/scripts/check_repo_test.py"', source)
        self.assertNotIn('python3 "$ROOT/scripts/bin_cache_test.py"', source)
        self.assertNotIn('python3 "$ROOT/scripts/bin_bootstrap_test.py"', source)


if __name__ == "__main__":
    unittest.main()
