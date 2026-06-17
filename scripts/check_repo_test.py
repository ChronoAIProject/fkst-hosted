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
from unittest import mock
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
check_repo_ingress = check_repo.check_repo_ingress


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


class OwnershipGateClaimOwnerGuardTest(unittest.TestCase):
    def violation_lines(self, source: str) -> list[int]:
        return check_repo.ownership_gate_defaulting_bot_login_lines(source)

    def test_flags_defaulting_getter_inside_pr_review_ownership_gate(self) -> None:
        source = """
function M.verify_pr_review_issue_claim(dept, repo, issue_number, current_issue, proposal_id)
  local owner = M.trusted_bot_login()
  return M.is_self_owned_issue(current_issue, owner)
end
"""
        self.assertEqual(self.violation_lines(source), [3])

    def test_allows_claim_owner_inside_pr_review_ownership_gate(self) -> None:
        source = """
function M.verify_pr_review_issue_claim(dept, repo, issue_number, current_issue, proposal_id)
  local owner = M.claim_owner()
  return M.is_self_owned_issue(current_issue, owner)
end

function M.trusted_bot_login()
  return "fkst-test-bot"
end
"""
        self.assertEqual(self.violation_lines(source), [])

    def test_ignores_comments_and_other_functions(self) -> None:
        source = """
function M.verify_pr_review_issue_claim(dept, repo, issue_number, current_issue, proposal_id)
  -- M.trusted_bot_login()
  local text = "M.trusted_bot_login()"
  return M.claim_owner()
end

function M.audit_trusted_bot()
  return M.trusted_bot_login()
end
"""
        self.assertEqual(self.violation_lines(source), [])


class ScopedFileWatchIngressGuardTest(unittest.TestCase):
    def violation(self, rel_path: str, source: str) -> str | None:
        root = Path("/repo")
        return check_repo_ingress.scoped_file_watch_ingress_violation(
            root,
            root / rel_path,
            source,
            check_repo.rel,
        )

    def test_allows_package_owned_ingress_glob(self) -> None:
        source = """
return {
  type = "file_watch",
  glob = ".fkst/ingress/github-proxy/issue-comment-request/*.json",
  produces = "issue_comment_request",
}
"""
        self.assertIsNone(
            self.violation(
                "packages/github-proxy/raisers/issue_comment_request_ingress.lua",
                source,
            )
        )

    def test_rejects_cross_package_ingress_glob(self) -> None:
        source = """
return {
  type = "file_watch",
  glob = ".fkst/ingress/consensus/issue-comment-request/*.json",
  produces = "issue_comment_request",
}
"""
        violation = self.violation(
            "packages/github-proxy/raisers/issue_comment_request_ingress.lua",
            source,
        )

        self.assertIsNotNone(violation)
        self.assertIn(".fkst/ingress/github-proxy/issue-comment-request/*.json", violation or "")

    def test_rejects_unmatched_queue_segment(self) -> None:
        source = """
return {
  type = "file_watch",
  glob = ".fkst/ingress/github-proxy/comments/*.json",
  produces = "issue_comment_request",
}
"""
        violation = self.violation(
            "packages/github-proxy/raisers/issue_comment_request_ingress.lua",
            source,
        )

        self.assertIsNotNone(violation)
        self.assertIn("issue_comment_request", violation or "")


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
        self.assertIn('python3 -B "$ROOT/scripts/check_repo_fkst_layout.py"', source)
        self.assertIn('python3 -B "$ROOT/scripts/check_repo_test.py"', source)
        self.assertIn('python3 -B "$ROOT/scripts/check_repo_fkst_layout_test.py"', source)
        self.assertIn('python3 -B "$ROOT/scripts/bin_cache_test.py"', source)
        self.assertIn('python3 -B "$ROOT/scripts/bin_bootstrap_test.py"', source)
        self.assertIn('python3 -B "$ROOT/scripts/board_test.py"', source)
        self.assertIn('python3 -B "$ROOT/scripts/doctor_test.py"', source)
        self.assertNotIn('python3 "$ROOT/scripts/check_repo.py"', source)
        self.assertNotIn('python3 "$ROOT/scripts/check_repo_fkst_layout.py"', source)
        self.assertNotIn('python3 "$ROOT/scripts/check_repo_test.py"', source)
        self.assertNotIn('python3 "$ROOT/scripts/check_repo_fkst_layout_test.py"', source)
        self.assertNotIn('python3 "$ROOT/scripts/bin_cache_test.py"', source)
        self.assertNotIn('python3 "$ROOT/scripts/bin_bootstrap_test.py"', source)
        self.assertNotIn('python3 "$ROOT/scripts/board_test.py"', source)
        self.assertNotIn('python3 "$ROOT/scripts/doctor_test.py"', source)

    def test_package_runtime_view_is_regenerated_from_source_packages(self) -> None:
        source = self.source()

        self.assertIn('SOURCE_PACKAGES_ROOT="$ROOT/packages"', source)
        self.assertIn('LOCAL_PACKAGES_ROOT="$FKST_DIR/local-packages"', source)
        self.assertIn('EXTERNAL_PACKAGES_ROOT="$FKST_DIR/packages"', source)
        self.assertIn('ln -sfn ../packages "$LOCAL_PACKAGES_ROOT"', source)
        self.assertIn('for src_pkg in "$SOURCE_PACKAGES_ROOT"/*/; do', source)
        self.assertIn('pkg="$LOCAL_PACKAGES_ROOT/$name"', source)

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
            pkg = probe / "packages" / "oversized"
            scripts.mkdir(parents=True)
            pkg.mkdir(parents=True)

            for name in ("run.sh", "bin_bootstrap.sh", "check_repo.py", "check_repo_gh_git_adapter.py", "check_repo_ingress.py"):
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
    def test_line_limit_guard_scans_department_local_submodules(self) -> None:
        root = Path(__file__).resolve().parents[1]
        with tempfile.TemporaryDirectory() as tmp:
            probe = Path(tmp) / "repo"
            module_dir = probe / "packages" / "github-devloop" / "departments" / "observability"
            module_dir.mkdir(parents=True)
            (module_dir / "dashboard.lua").write_text("-- filler\n" * (check_repo.LINE_LIMIT + 1), encoding="utf-8")

            violations: list[str] = []
            warnings: list[str] = []
            check_repo.check_line_limit(probe, violations, warnings)

        self.assertEqual(
            violations,
            [
                "G1: packages/github-devloop/departments/observability/dashboard.lua has 1001 lines; limit is 1000",
            ],
        )
        self.assertEqual(warnings, [])

    def test_near_limit_source_file_warns_without_failing(self) -> None:
        root = Path(__file__).resolve().parents[1]
        with tempfile.TemporaryDirectory() as tmp:
            probe = Path(tmp) / "repo"
            script_dir = probe / "scripts"
            pkg = probe / "packages" / "near-limit"
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
            pkg = probe / "packages" / "oversized"
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


class ObservabilitySplitArchitectureTest(unittest.TestCase):
    def test_observability_source_is_split_into_department_responsibilities(self) -> None:
        root = Path(__file__).resolve().parents[1]
        package_root = root / "packages" / "github-devloop"
        module_dir = package_root / "departments" / "observability"
        expected_modules = {
            "common.lua",
            "dashboard.lua",
            "census.lua",
            "reaper.lua",
        }

        for name in sorted(expected_modules):
            path = module_dir / name
            self.assertTrue(path.is_file(), f"missing department-local observability module: {path}")
            self.assertLess(
                check_repo.line_count(path),
                check_repo.LINE_LIMIT - check_repo.LINE_WARNING_MARGIN,
                f"{path} must stay below the line-limit warning threshold",
            )

        core_observability = package_root / "core" / "observability.lua"
        self.assertLess(
            check_repo.line_count(core_observability),
            check_repo.LINE_LIMIT - check_repo.LINE_WARNING_MARGIN,
            "core/observability.lua must remain an orchestration layer, not a near-limit monolith",
        )


class RepositoryInterfaceContractTest(unittest.TestCase):
    def test_repository_checks_scan_fkst_packages_view(self) -> None:
        root = Path(__file__).resolve().parents[1]

        self.assertEqual(check_repo.packages_root(root), root / "packages")


class CrossPackageRequireTest(unittest.TestCase):
    def names(self, source: str, packages: list[str], current: str) -> list[str]:
        return check_repo.cross_package_require_names(source, set(packages), current)

    def test_flags_sibling_package_require(self) -> None:
        src = 'local x = require("github-proxy.core")\n'
        self.assertEqual(
            self.names(src, ["github-proxy", "consensus"], "consensus"),
            ["github-proxy"],
        )

    def test_flags_string_call_sibling_package_require(self) -> None:
        src = 'local a = require "github-proxy.core"\nlocal b = require[[github-proxy.util]]\n'
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

    def test_ignores_require_text_inside_comments_and_strings(self) -> None:
        src = """
-- local x = require("github-proxy.core")
local text = 'require("github-proxy.core")'
local block = [[
  require("github-proxy.core")
]]
local real = require("consensus.thing")
"""
        self.assertEqual(
            self.names(src, ["github-proxy", "consensus"], "consensus"),
            [],
        )


class ConvergenceBudgetGuardTest(unittest.TestCase):
    def violations(self, loop_source: str, review_source: str | None = None) -> list[str]:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            loop = root / "packages" / "github-devloop" / "departments" / "loop"
            review = root / "packages" / "github-devloop" / "departments" / "review_loop"
            loop.mkdir(parents=True)
            review.mkdir(parents=True)
            loop.joinpath("main.lua").write_text(loop_source, encoding="utf-8")
            review.joinpath("main.lua").write_text(review_source or "", encoding="utf-8")
            violations: list[str] = []
            check_repo.check_convergence_budget_caps(root, violations)
            return violations

    def test_flags_loop_budget_without_stable_proposal_round_facts(self) -> None:
        source = """
local facts = core.converge_round_facts(current.comments, proposal_id, base_version, sr_digest)
local round = math.max(payload.round, core.max_converge_round(facts))
local budget_round = math.max(round, core.converge_boundary_budget_round(current.comments, proposal_id, payload.narrowed_question, payload.angle_digests))
local hit_round_cap = budget_round >= core.max_converge_rounds()
"""
        violations = self.violations(source)
        self.assertEqual(len(violations), 1)
        self.assertIn("convergence round counter must derive from stable boundary-preserving core.converge_round_facts_for_proposal_boundary() facts", violations[0])

    def test_flags_loop_budget_without_boundary_preserving_budget(self) -> None:
        source = """
local facts = core.converge_round_facts_for_proposal_boundary(current.comments, proposal_id, payload.narrowed_question, payload.angle_digests)
local round = math.max(payload.round, core.max_converge_round(facts))
local budget_round = math.max(round, core.converge_budget_round(current.comments, proposal_id))
local hit_round_cap = budget_round >= core.max_converge_rounds()
"""
        violations = self.violations(source)
        self.assertEqual(len(violations), 1)
        self.assertIn("convergence cap must use boundary-preserving core.converge_boundary_budget_round() budget", violations[0])

    def test_allows_loop_budget_with_stable_boundary_preserving_round_facts(self) -> None:
        source = """
local facts = core.converge_round_facts_for_proposal_boundary(current.comments, proposal_id, payload.narrowed_question, payload.angle_digests)
local round = math.max(payload.round, core.max_converge_round(facts))
local budget_round = math.max(round, core.converge_boundary_budget_round(current.comments, proposal_id, payload.narrowed_question, payload.angle_digests))
local hit_round_cap = budget_round >= core.max_converge_rounds()
"""
        self.assertEqual(self.violations(source), [])


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

    def test_paren_call_saga_department_is_detected(self) -> None:
        # `.department(...)` (paren spelling) must be recognized as saga-shaped so a
        # paren-form migration cannot silently remain on the allowlist and false-pass.
        source = 'local saga = require("std.saga")\nreturn saga.department({done = d, act = a, consumes = {"q"}})\n'
        allowlist = {"packages/example/departments/dept/main.lua"}
        self.assertIn("saga-shaped department remains on saga-handler allowlist", self.violations(source, allowlist)[0])

    def test_allowlist_growth_relative_to_base_fails(self) -> None:
        source = 'function pipeline(event)\n  return event\nend\n'
        allowlist = {
            "packages/example/departments/dept/main.lua",
            "packages/example/departments/new/main.lua",
        }
        base = {"packages/example/departments/dept/main.lua"}
        violations = check_repo.saga_handler_ratchet_violations({
            "packages/example/departments/dept/main.lua": source,
            "packages/example/departments/new/main.lua": source,
        }, allowlist, base)
        self.assertIn("grows saga-handler allowlist relative to dev", violations[0])

    def test_allowlist_equal_or_shrunk_relative_to_base_passes(self) -> None:
        free_form = 'function pipeline(event)\n  return event\nend\n'
        saga_shaped = 'local saga = require("std.saga")\nreturn saga.department{done = d, act = a, consumes = {"q"}}\n'
        base = {
            "packages/example/departments/dept/main.lua",
            "packages/example/departments/old/main.lua",
        }
        self.assertEqual(
            check_repo.saga_handler_ratchet_violations({
                "packages/example/departments/dept/main.lua": free_form,
                "packages/example/departments/old/main.lua": free_form,
            }, set(base), base),
            [],
        )
        self.assertEqual(
            check_repo.saga_handler_ratchet_violations(
                {
                    "packages/example/departments/dept/main.lua": free_form,
                    "packages/example/departments/old/main.lua": saga_shaped,
                },
                {"packages/example/departments/dept/main.lua"},
                base,
            ),
            [],
        )

    def test_missing_dev_base_is_violation_not_warning(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            dept = root / "packages" / "example" / "departments" / "dept"
            dept.mkdir(parents=True)
            (dept / "main.lua").write_text(
                'function pipeline(event)\n  return event\nend\n',
                encoding="utf-8",
            )
            migration = root / "migration"
            migration.mkdir()
            (migration / "saga-handler.allowlist").write_text(
                "packages/example/departments/dept/main.lua\n",
                encoding="utf-8",
            )

            violations: list[str] = []
            warnings: list[str] = []
            with mock.patch.object(check_repo, "saga_allowlist_at_dev_base", return_value=("unresolved", None)):
                check_repo.check_saga_handler_ratchet(root, violations, warnings)

        self.assertEqual(warnings, [])
        self.assertIn("cannot resolve dev base allowlist", violations[0])

    def test_first_introduction_without_base_allowlist_passes(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            dept = root / "packages" / "example" / "departments" / "dept"
            dept.mkdir(parents=True)
            (dept / "main.lua").write_text(
                'function pipeline(event)\n  return event\nend\n',
                encoding="utf-8",
            )
            migration = root / "migration"
            migration.mkdir()
            (migration / "saga-handler.allowlist").write_text(
                "packages/example/departments/dept/main.lua\n",
                encoding="utf-8",
            )

            violations: list[str] = []
            warnings: list[str] = []
            with mock.patch.object(check_repo, "saga_allowlist_at_dev_base", return_value=("absent", None)):
                check_repo.check_saga_handler_ratchet(root, violations, warnings)

        self.assertEqual(warnings, [])
        self.assertEqual(violations, [])


class GhGitAdapterRatchetTest(unittest.TestCase):
    def messages(self, sources: dict[str, str], allowlist: dict[str, set[str]] | None = None) -> list[str]:
        return check_repo.gh_git_adapter.ratchet_messages(sources, allowlist or {})

    def test_builder_literal_is_flagged(self) -> None:
        messages = self.messages({
            "packages/example/core.lua": 'return "gh issue list"\n',
        })

        self.assertEqual(len(messages), 1)
        self.assertIn("packages/example/core.lua constructs a new gh/git command head 'gh issue'", messages[0])

    def test_log_message_literal_is_excluded(self) -> None:
        messages = self.messages({
            "packages/example/core.lua": 'log.info("git merge done")\n',
        })

        self.assertEqual(messages, [])

    def test_exec_wrapper_context_labels_are_excluded(self) -> None:
        source = (
            "run_cmd(core.git_fetch_branch_cmd('origin', b), 60, 'git rollup fetch')\n"
            "gh_exec(M.gh_issue_blocked_by_cmd(r,n), 30, 'gh blockedBy view')\n"
        )

        self.assertEqual(check_repo.gh_git_adapter.command_heads(source), set())

    def test_literal_commands_remain_flagged(self) -> None:
        source = (
            "function M.gh_issue_list_cmd(r)\n"
            "  return 'gh issue list --repo ' .. r\n"
            "end\n"
            "local cmd = 'git push origin ' .. ref\n"
            "gh_exec('gh api graphql', 30, 'gh label context')\n"
        )

        self.assertEqual(check_repo.gh_git_adapter.command_heads(source), {"gh issue", "git push", "gh api"})

    def test_dotted_receiver_and_multiline_wrapper_calls(self) -> None:
        # Stable synthetic fixture (no real-file dependency) pinning the parser across
        # dotted-receiver wrappers (M./core. prefixes) and multiline call syntax: a label
        # argument is excluded, while an arg-0 inline command stays flagged in both shapes.
        source = (
            "core.gh_exec(M.gh_issue_blocked_by_cmd(r, n), 30, 'gh blockedBy view')\n"
            "M.gh_exec('gh pr merge --admin', 30, 'merge context')\n"
            "run_git(\n"
            "  core.git_push_cmd(worktree, branch),\n"
            "  120,\n"
            "  'git resolved branch sync push'\n"
            ")\n"
            "run_cmd(\n"
            "  'git fetch origin ' .. branch,\n"
            "  60,\n"
            "  'git fetch context'\n"
            ")\n"
        )

        self.assertEqual(check_repo.gh_git_adapter.command_heads(source), {"gh pr", "git fetch"})

    def test_cited_context_label_files_no_longer_report_label_only_heads(self) -> None:
        root = Path(__file__).resolve().parents[1]
        sources = {
            rel: (root / rel).read_text(encoding="utf-8")
            for rel in (
                "packages/github-devloop/departments/rollup_scan/main.lua",
                "packages/github-proxy/core/blocked_by.lua",
                "packages/github-devloop/core/ensure_repo.lua",
                "packages/github-devloop/core/substrate_ref.lua",
            )
        }
        heads_by_file = check_repo.gh_git_adapter.command_heads_by_file(sources)

        self.assertNotIn("git rollup", heads_by_file.get("packages/github-devloop/departments/rollup_scan/main.lua", set()))
        self.assertNotIn("gh rollup", heads_by_file.get("packages/github-devloop/departments/rollup_scan/main.lua", set()))
        self.assertNotIn("gh blockedBy", heads_by_file.get("packages/github-proxy/core/blocked_by.lua", set()))
        self.assertNotIn("git integration", heads_by_file.get("packages/github-devloop/core/ensure_repo.lua", set()))
        self.assertNotIn("gh substrate-ref", heads_by_file.get("packages/github-devloop/core/substrate_ref.lua", set()))
        self.assertNotIn("git substrate-ref", heads_by_file.get("packages/github-devloop/core/substrate_ref.lua", set()))
        self.assertNotIn("git stale", heads_by_file.get("packages/github-devloop/core/substrate_ref.lua", set()))

    def test_env_cd_absolute_path_shell_c_and_concat_are_normalized(self) -> None:
        source = (
            'local a = "FOO=1 cd /tmp && /usr/local/bin/git" .. " -C /repo status --short"\n'
            'local b = "bash -c \\"gh pr view\\""\n'
            'local c = "GH_HOST=github.com gh" .. " issue view 1"\n'
        )
        heads = check_repo.gh_git_adapter.command_heads(source)

        self.assertEqual(heads, {"git status", "gh pr", "gh issue"})

    def test_new_head_in_allowlisted_file_fails(self) -> None:
        messages = self.messages(
            {"packages/example/core.lua": 'return "gh issue list"\nreturn "git status"\n'},
            {"packages/example/core.lua": {"gh issue"}},
        )

        self.assertEqual(len(messages), 1)
        self.assertIn("constructs a new gh/git command head 'git status'", messages[0])

    def test_stale_head_forces_allowlist_shrink(self) -> None:
        messages = self.messages(
            {"packages/example/core.lua": 'return "gh issue list"\n'},
            {"packages/example/core.lua": {"gh issue", "git status"}},
        )

        self.assertEqual(len(messages), 1)
        self.assertIn("no longer constructs 'git status'; update its entry", messages[0])

    def test_root_std_non_adapter_flagged_and_std_github_exempt(self) -> None:
        sources = {
            "std/helpers.lua": 'return "git status"\n',
            "std/github/exec.lua": 'return "gh issue list"\n',
            "std/github.lua": 'return "gh pr view"\n',
        }

        messages = self.messages(sources)

        self.assertEqual(len(messages), 1)
        self.assertIn("std/helpers.lua constructs a new gh/git command head 'git status'", messages[0])

    def test_exec_argv_raw_heads_are_flagged_outside_adapters(self) -> None:
        source = (
            'exec_argv({ argv = { "gh", "issue", "view", tostring(n) }, timeout = 30 })\n'
            'exec_argv({ timeout = 30, argv = { "git", "-C", repo, "status", "--short" } })\n'
        )
        heads = check_repo.gh_git_adapter.command_heads(source)

        self.assertEqual(heads, {"gh issue", "git status"})

    def test_exec_argv_raw_heads_respect_adapter_exemption(self) -> None:
        messages = self.messages({
            "std/github/issue.lua": 'exec_argv({ argv = { "gh", "issue", "view", "1" } })\n',
            "std/git/exec.lua": 'exec_argv({ argv = { "git", "status" } })\n',
            "std/helpers.lua": 'exec_argv({ argv = { "git", "status" } })\n',
        })

        self.assertEqual(len(messages), 1)
        self.assertIn("std/helpers.lua constructs a new gh/git command head 'git status'", messages[0])

    def test_check_repo_wrapper_loads_allowlist_and_prefixes_violations(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            package = root / "packages" / "example"
            migration = root / "migration"
            package.mkdir(parents=True)
            migration.mkdir()
            (package / "core.lua").write_text('return "gh issue list"\n', encoding="utf-8")
            (migration / "gh-git-adapter.allowlist").write_text(
                "packages/example/core.lua:\n"
                "  - gh issue\n",
                encoding="utf-8",
            )

            violations: list[str] = []
            check_repo.check_gh_git_adapter_ratchet(root, violations)
            self.assertEqual(violations, [])

            (package / "core.lua").write_text('return "gh pr view"\n', encoding="utf-8")
            check_repo.check_gh_git_adapter_ratchet(root, violations)

        self.assertEqual(len(violations), 2)
        self.assertTrue(all(message.startswith("G-ADAPTER: ") for message in violations))
        self.assertIn("constructs a new gh/git command head 'gh pr'", violations[0])
        self.assertIn("no longer constructs 'gh issue'", violations[1])


if __name__ == "__main__":
    unittest.main()
