#!/usr/bin/env python3
"""Unit tests for repository guard helpers."""

from __future__ import annotations

import importlib.util
import os
import shutil
import subprocess
import tempfile
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


class ErrorClassPrefixGuardTest(unittest.TestCase):
    def warning_lines(self, source: str) -> list[int]:
        return check_repo.unclassified_error_call_lines(source)

    def test_warns_error_without_class_prefix(self) -> None:
        source = """
error("github-devloop: failed without narrow class")
"""
        self.assertEqual(self.warning_lines(source), [2])

    def test_allows_error_with_class_prefix(self) -> None:
        source = """
error("github-devloop: gh-view-failed: details")
"""
        self.assertEqual(self.warning_lines(source), [])

    def test_ignores_comments_and_dynamic_messages(self) -> None:
        source = """
-- error("github-devloop: failed without narrow class")
error(prefix .. detail)
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
        self.assertIn('python3 -B "$ROOT/scripts/board_test.py"', source)
        self.assertIn('python3 -B "$ROOT/scripts/doctor_test.py"', source)
        self.assertNotIn('python3 "$ROOT/scripts/check_repo.py"', source)
        self.assertNotIn('python3 "$ROOT/scripts/check_repo_test.py"', source)
        self.assertNotIn('python3 "$ROOT/scripts/bin_cache_test.py"', source)
        self.assertNotIn('python3 "$ROOT/scripts/bin_bootstrap_test.py"', source)
        self.assertNotIn('python3 "$ROOT/scripts/board_test.py"', source)
        self.assertNotIn('python3 "$ROOT/scripts/doctor_test.py"', source)

    def test_full_test_blocks_on_repository_check_before_engine_resolution(self) -> None:
        source = self.source()

        self.assertIn("elif ! _chk_out=\"$(cmd_check 2>&1)\"; then", source)
        self.assertIn("printf '%s\\n' \"$_chk_out\"; exit 1", source)
        self.assertLess(source.index("cmd_check"), source.index("resolve_bin; ensure_fresh_bin; cmd_test"))

    def test_full_test_fails_on_g1_before_bin_resolution(self) -> None:
        root = Path(__file__).resolve().parents[1]
        with tempfile.TemporaryDirectory() as tmp:
            probe = Path(tmp) / "repo"
            scripts = probe / "scripts"
            pkg = probe / ".fkst" / "packages" / "oversized"
            scripts.mkdir(parents=True)
            pkg.mkdir(parents=True)

            for name in ("run.sh", "bin_bootstrap.sh", "check_repo.py"):
                shutil.copy2(root / "scripts" / name, scripts / name)
            for name in ("check_repo_test.py", "bin_cache_test.py", "bin_bootstrap_test.py", "board_test.py", "doctor_test.py"):
                (scripts / name).write_text("#!/usr/bin/env python3\nraise SystemExit(0)\n", encoding="utf-8")

            core_lines = [
                "local M = {}",
                "function M.persistence_class() return \"stateless_adapter\" end",
                "return M",
            ]
            core_lines.extend("-- filler" for _ in range(check_repo.LINE_LIMIT + 1 - len(core_lines)))
            (pkg / "core.lua").write_text("\n".join(core_lines) + "\n", encoding="utf-8")

            env = os.environ.copy()
            env["BIN"] = str(probe / "missing-fkst-framework")
            result = subprocess.run(
                ["/bin/bash", "scripts/run.sh", "test"],
                cwd=probe,
                env=env,
                text=True,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                check=False,
            )

        combined = result.stdout + result.stderr
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("repository check failed:", combined)
        self.assertIn("G1: packages/oversized/core.lua has 1001 lines; limit is 1000", combined)
        self.assertNotIn("explicit BIN is not executable", combined)


class LineLimitGuardTest(unittest.TestCase):
    def test_near_limit_source_file_warns_without_failing(self) -> None:
        root = Path(__file__).resolve().parents[1]
        with tempfile.TemporaryDirectory() as tmp:
            probe = Path(tmp) / "repo"
            script_dir = probe / "scripts"
            pkg = probe / ".fkst" / "packages" / "near-limit"
            script_dir.mkdir(parents=True)
            pkg.mkdir(parents=True)
            (script_dir / "helper.py").write_text("print('ok')\n", encoding="utf-8")
            (pkg / "core.lua").write_text("-- filler\n" * check_repo.LINE_WARNING_MARGIN, encoding="utf-8")

            violations: list[str] = []
            warnings: list[str] = []
            old_threshold = os.environ.get("FKST_G1_LINE_WARNING_THRESHOLD")
            os.environ["FKST_G1_LINE_WARNING_THRESHOLD"] = str(check_repo.LINE_WARNING_MARGIN)
            try:
                check_repo.check_line_limit(probe, violations, warnings)
            finally:
                if old_threshold is None:
                    os.environ.pop("FKST_G1_LINE_WARNING_THRESHOLD", None)
                else:
                    os.environ["FKST_G1_LINE_WARNING_THRESHOLD"] = old_threshold

        self.assertEqual(violations, [])
        self.assertEqual(
            warnings,
            [
                "G1: packages/near-limit/core.lua has 50 lines; warning threshold is 50; hard limit is 1000",
            ],
        )

    def test_over_limit_source_file_fails_without_duplicate_warning(self) -> None:
        root = Path(__file__).resolve().parents[1]
        with tempfile.TemporaryDirectory() as tmp:
            probe = Path(tmp) / "repo"
            pkg = probe / ".fkst" / "packages" / "oversized"
            pkg.mkdir(parents=True)
            (pkg / "core.lua").write_text("-- filler\n" * (check_repo.LINE_LIMIT + 1), encoding="utf-8")

            violations: list[str] = []
            warnings: list[str] = []
            check_repo.check_line_limit(probe, violations, warnings)

        self.assertEqual(
            violations,
            [
                "G1: packages/oversized/core.lua has 1001 lines; limit is 1000",
            ],
        )
        self.assertEqual(warnings, [])


class RepositoryInterfaceContractTest(unittest.TestCase):
    def test_repository_checks_scan_fkst_packages_view(self) -> None:
        root = Path(__file__).resolve().parents[1]

        self.assertEqual(check_repo.packages_root(root), root / ".fkst" / "packages")


class CrossPackageRequireTest(unittest.TestCase):
    def names(self, source: str, packages: list[str], current: str) -> list[str]:
        return check_repo.cross_package_require_names(source, set(packages), current)

    def test_flags_sibling_package_require(self) -> None:
        src = 'local x = require("github-proxy.core")\n'
        self.assertEqual(
            self.names(src, ["github-proxy", "consensus"], "consensus"),
            ["github-proxy"],
        )

    def test_allows_std_core_departments_fkst(self) -> None:
        src = (
            'require("std.saga") require("core") require("core.markers") '
            'require("departments.foo") require("fkst")\n'
        )
        self.assertEqual(self.names(src, ["github-proxy", "consensus"], "consensus"), [])

    def test_self_reference_is_not_cross_package(self) -> None:
        src = 'require("consensus.thing")\n'
        self.assertEqual(self.names(src, ["consensus"], "consensus"), [])


class SagaHandlerRatchetTest(unittest.TestCase):
    def violations(self, source: str, allowlist: set[str]) -> list[str]:
        return check_repo.saga_handler_ratchet_violations({
            "packages/example/departments/dept/main.lua": source,
        }, allowlist)

    def test_saga_shaped_department_not_on_allowlist_passes(self) -> None:
        source = 'local saga = require("std.saga")\nreturn saga.department{done = d, act = a, consumes = {"q"}}\n'
        self.assertEqual(self.violations(source, set()), [])

    def test_free_form_department_on_allowlist_passes(self) -> None:
        source = 'function pipeline(event)\n  return event\nend\n'
        allowlist = {"packages/example/departments/dept/main.lua"}
        self.assertEqual(self.violations(source, allowlist), [])

    def test_free_form_department_not_on_allowlist_fails(self) -> None:
        source = 'pipeline = function(event)\n  return event\nend\n'
        self.assertEqual(len(self.violations(source, set())), 1)
        self.assertIn("free-form department not on saga-handler allowlist", self.violations(source, set())[0])

    def test_saga_shaped_department_on_allowlist_fails(self) -> None:
        source = 'return require("std.saga").department{done = d, act = a, consumes = {"q"}}\n'
        allowlist = {"packages/example/departments/dept/main.lua"}
        self.assertIn("saga-shaped department remains on saga-handler allowlist", self.violations(source, allowlist)[0])

    def test_saga_shaped_department_with_leftover_pipeline_fails(self) -> None:
        source = 'local saga = require("std.saga")\npipeline = function() end\nreturn saga.department{done = d, act = a, consumes = {"q"}}\n'
        self.assertIn("still defines free-form top-level pipeline", self.violations(source, set())[0])


if __name__ == "__main__":
    unittest.main()
