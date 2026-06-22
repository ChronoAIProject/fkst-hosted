#!/usr/bin/env python3
"""Unit tests for the Lua coverage shrink-only ratchet."""

from __future__ import annotations

import importlib.util
import json
import subprocess
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


scripts = Path(__file__).resolve().parent
coverage = load_module("check_repo_coverage", scripts / "check_repo_coverage.py")


def git(root: Path, *args: str) -> str:
    result = subprocess.run(
        ["git", *args],
        cwd=root,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        check=False,
    )
    if result.returncode != 0:
        raise AssertionError(f"git {' '.join(args)} failed: {result.stderr}")
    return result.stdout.strip()


class CoverageRatchetTest(unittest.TestCase):
    def key(self, file: str = "packages/example/core.lua", line: int = 2, digest: str = "abcdef12"):
        return coverage.CoverageKey(file, line, digest)

    def test_lua_candidate_classifier_excludes_structurally_non_executable_lines(self) -> None:
        excluded = [
            "})",
            "))",
            "}),",
            "  )),",
            "} )",
            "local function bounded(value, limit)",
            "function M.foo(a, b)",
            "local f = function(x)",
            "return function(event)",
            "'schema text',",
            '"a string",',
            '"^%d+$"',
        ]

        for line in excluded:
            with self.subTest(line=line):
                self.assertFalse(coverage.is_candidate_executable_lua_line(line))

    def test_lua_candidate_classifier_keeps_executable_behavior_lines(self) -> None:
        included = [
            "local function f() return 1 end",
            'error("x: y")',
            "return bounded(a, b)",
            'x = foo("bar")',
            "if not ok then",
            "log_skip(reason, event)",
        ]

        for line in included:
            with self.subTest(line=line):
                self.assertTrue(coverage.is_candidate_executable_lua_line(line))

    def test_new_uncovered_production_line_fails_with_source_text(self) -> None:
        uncovered = {
            self.key(): coverage.UncoveredLine(self.key(), "return missing_branch()"),
        }

        messages = coverage.ratchet_messages(uncovered, set())

        self.assertEqual(len(messages), 1)
        self.assertIn("packages/example/core.lua:2:return missing_branch()", messages[0])
        self.assertIn("not in migration/coverage-uncovered.allowlist", messages[0])

    def test_allowlisted_uncovered_line_passes(self) -> None:
        uncovered = {
            self.key(): coverage.UncoveredLine(self.key(), "return missing_branch()"),
        }

        self.assertEqual(coverage.ratchet_messages(uncovered, {self.key()}), [])

    def test_stale_allowlist_entry_is_advisory(self) -> None:
        messages = coverage.ratchet_messages({}, {self.key()})
        stale_messages = coverage.stale_allowlist_messages({}, {self.key()})

        self.assertEqual(messages, [])
        self.assertEqual(len(stale_messages), 1)
        self.assertIn("is no longer uncovered; prune the stale entry", stale_messages[0])

    def test_allowlist_growth_relative_to_base_fails(self) -> None:
        old_key = self.key(line=1, digest="11111111")
        current = {old_key, self.key()}
        base = {old_key}

        messages = coverage.ratchet_messages({}, current, base, "integration")

        self.assertIn("grows migration/coverage-uncovered.allowlist relative to integration", messages[-1])

    def test_stale_allowlist_entry_does_not_mask_allowlist_growth(self) -> None:
        current = {self.key()}
        messages = coverage.ratchet_messages({}, current, set(), "integration")

        self.assertEqual(len(messages), 1)
        self.assertIn("grows migration/coverage-uncovered.allowlist relative to integration", messages[0])

    def test_shifted_uncovered_line_is_tolerated_by_content(self) -> None:
        # An already-allowlisted uncovered line that shifts to a new line number
        # (identical content) is the SAME line, not a new one: no regen required.
        shifted = self.key(line=337, digest="cccccccc")
        allow_at_old_line = self.key(line=339, digest="cccccccc")
        uncovered = {shifted: coverage.UncoveredLine(shifted, 'error("git worktree add failed")')}

        self.assertEqual(coverage.ratchet_messages(uncovered, {allow_at_old_line}), [])

    def test_line_move_is_not_growth_relative_to_base(self) -> None:
        # Same content at a new line is not allowlist growth (a move, not an add).
        current = {self.key(line=337, digest="cccccccc")}
        base = {self.key(line=339, digest="cccccccc")}

        self.assertEqual(coverage.ratchet_messages({}, current, base, "integration"), [])

    def test_covered_line_becoming_uncovered_is_still_caught(self) -> None:
        # Content absent from the allowlist (a real coverage regression or new
        # uncovered code) is still flagged even though matching ignores line.
        novel = self.key(line=42, digest="deadbeef")
        uncovered = {novel: coverage.UncoveredLine(novel, "return new_uncovered()")}

        messages = coverage.ratchet_messages(uncovered, {self.key(line=339, digest="cccccccc")})

        self.assertEqual(len(messages), 1)
        self.assertIn("not in migration/coverage-uncovered.allowlist", messages[0])

    def test_duplicate_content_match_is_count_aware(self) -> None:
        # Two identical-content allowlist entries cover two uncovered duplicates;
        # a third novel duplicate of the same content is still flagged.
        a = self.key(line=10, digest="dup00000")
        b = self.key(line=20, digest="dup00000")
        c = self.key(line=30, digest="dup00000")
        uncovered = {
            a: coverage.UncoveredLine(a, 'error("x")'),
            b: coverage.UncoveredLine(b, 'error("x")'),
            c: coverage.UncoveredLine(c, 'error("x")'),
        }
        allowlist = {self.key(line=11, digest="dup00000"), self.key(line=21, digest="dup00000")}

        messages = coverage.ratchet_messages(uncovered, allowlist)

        self.assertEqual(len(messages), 1)

    def test_duplicate_content_growth_is_count_aware(self) -> None:
        # Adding a third allowlist entry for a content with only two in base grows.
        current = {
            self.key(line=10, digest="dup00000"),
            self.key(line=20, digest="dup00000"),
            self.key(line=30, digest="dup00000"),
        }
        base = {self.key(line=11, digest="dup00000"), self.key(line=21, digest="dup00000")}

        messages = coverage.ratchet_messages({}, current, base, "integration")

        self.assertEqual(len(messages), 1)
        self.assertIn("grows", messages[0])

    def test_moved_allowlist_entry_is_not_stale(self) -> None:
        # An allowlist entry whose content is still uncovered (at a new line) is
        # not stale; only content that is no longer uncovered anywhere is stale.
        allowlist = {self.key(line=339, digest="cccccccc")}
        uncovered = {
            self.key(line=337, digest="cccccccc"): coverage.UncoveredLine(
                self.key(line=337, digest="cccccccc"), 'error("git worktree add failed")'
            )
        }

        self.assertEqual(coverage.stale_allowlist_messages(uncovered, allowlist), [])

    def test_engine_file_metadata_is_authoritative_source(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            artifact = Path(tmp) / "coverage.json"
            artifact.write_text(
                json.dumps({
                    "schema": "fkst.lua.coverage.v1",
                    "files": [{
                        "file": "packages/example/core.lua",
                        "coverable_lines": [
                            {"line": 1, "normalized_line_hash": "11111111", "text": "local M = {}", "covered": True},
                            {"line": 2, "normalized_line_hash": "abcdef12", "text": "return missing_branch()", "covered": False},
                        ],
                    }],
                }),
                encoding="utf-8",
            )

            uncovered = coverage.uncovered_from_artifact(artifact)

        self.assertEqual(set(uncovered), {self.key()})
        self.assertEqual(uncovered[self.key()].text, "return missing_branch()")

    def test_write_current_uncovered_writes_stable_sorted_allowlist(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            artifact = root / "coverage.json"
            artifact.write_text(
                json.dumps({
                    "schema": "fkst.lua.coverage.v1",
                    "files": [
                        {
                            "file": "libraries/forge/zeta.lua",
                            "missing_lines": [{
                                "line": 7,
                                "normalized_line_hash": "bbbbbbbb",
                                "text": "return zeta()",
                            }],
                        },
                        {
                            "file": "libraries/contract/strings.lua",
                            "missing_lines": [{
                                "line": 3,
                                "normalized_line_hash": "dddddddd",
                                "text": "return shared()",
                            }],
                        },
                        {
                            "file": "packages/example/tests/core_test.lua",
                            "missing_lines": [{
                                "line": 1,
                                "normalized_line_hash": "cccccccc",
                                "text": "error('test')",
                            }],
                        },
                        {
                            "file": "packages/example/core.lua",
                            "missing_lines": [{
                                "line": 2,
                                "normalized_line_hash": "abcdef12",
                                "text": "return missing_branch()",
                            }],
                        },
                    ],
                }),
                encoding="utf-8",
            )
            allowlist = root / "migration" / "coverage-uncovered.allowlist"

            count = coverage.write_current_uncovered(artifact, allowlist)
            first = allowlist.read_text(encoding="utf-8")
            count_again = coverage.write_current_uncovered(artifact, allowlist)
            second = allowlist.read_text(encoding="utf-8")

        self.assertEqual(count, 3)
        self.assertEqual(count_again, 3)
        self.assertEqual(first, second)
        self.assertEqual(
            [json.loads(line) for line in first.splitlines()],
            [
                {
                    "file": "libraries/contract/strings.lua",
                    "line": 3,
                    "normalized_line_hash": "dddddddd",
                    "reason": "baseline",
                },
                {
                    "file": "libraries/forge/zeta.lua",
                    "line": 7,
                    "normalized_line_hash": "bbbbbbbb",
                    "reason": "baseline",
                },
                {
                    "file": "packages/example/core.lua",
                    "line": 2,
                    "normalized_line_hash": "abcdef12",
                    "reason": "baseline",
                },
            ],
        )

    def test_write_current_uncovered_from_covered_sets_includes_uncovered_files(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "packages" / "example").mkdir(parents=True)
            (root / "packages" / "example" / "core.lua").write_text(
                "\n".join([
                    "local M = {}",
                    "function M.covered()",
                    "  return 1",
                    "end",
                    "function M.missing()",
                    "  return 2",
                    "end",
                    "return M",
                ]) + "\n",
                encoding="utf-8",
            )
            (root / "packages" / "example" / "unused.lua").write_text(
                "\n".join([
                    "local M = {}",
                    "function M.unused()",
                    "  return 3",
                    "end",
                    "return M",
                ]) + "\n",
                encoding="utf-8",
            )
            allowlist = root / "migration" / "coverage-uncovered.allowlist"

            count = coverage.write_current_uncovered_from_covered_sets(
                {"packages/example/core.lua": {1, 2, 3, 8}},
                allowlist,
                root,
            )
            entries = [json.loads(line) for line in allowlist.read_text(encoding="utf-8").splitlines()]

        self.assertGreaterEqual(count, 3)
        self.assertIn(
            {
                "file": "packages/example/core.lua",
                "line": 6,
                "normalized_line_hash": coverage.normalized_source_hash("  return 2"),
                "reason": "baseline",
            },
            entries,
        )
        self.assertIn(
            {
                "file": "packages/example/unused.lua",
                "line": 3,
                "normalized_line_hash": coverage.normalized_source_hash("  return 3"),
                "reason": "baseline",
            },
            entries,
        )

    def test_write_canonical_coverage_json_writes_authoritative_artifact(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "packages" / "example").mkdir(parents=True)
            (root / "libraries" / "forge").mkdir(parents=True)
            (root / "libraries" / "contract").mkdir(parents=True)
            (root / "packages" / "example" / "core.lua").write_text(
                "local M = {}\nfunction M.covered()\n  return 1\nend\nreturn M\n",
                encoding="utf-8",
            )
            (root / "libraries" / "forge" / "shared.lua").write_text("return {}\n", encoding="utf-8")
            (root / "libraries" / "contract" / "strings.lua").write_text("return {}\n", encoding="utf-8")
            output = root / "coverage.json"

            count = coverage.write_canonical_coverage_json(
                {
                    "libraries/contract/strings.lua": {1},
                    "packages/example/core.lua": {1, 2, 3, 5},
                    "libraries/forge/shared.lua": {1},
                },
                output,
                root,
            )
            data = json.loads(output.read_text(encoding="utf-8"))

            self.assertEqual(count, 3)
            self.assertEqual(data["schema"], "fkst.lua.coverage.v1")
            self.assertEqual(
                [item["file"] for item in data["files"]],
                ["libraries/contract/strings.lua", "libraries/forge/shared.lua", "packages/example/core.lua"],
            )
            files_by_path = {item["file"]: item for item in data["files"]}
            core_lines = files_by_path["packages/example/core.lua"]["coverable_lines"]
            self.assertIn(
                {
                    "line": 3,
                    "normalized_line_hash": coverage.normalized_source_hash("  return 1"),
                    "text": "return 1",
                    "covered": True,
                },
                core_lines,
            )

    def test_repository_messages_loads_jsonl_allowlist(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "migration").mkdir()
            (root / "migration" / "coverage-uncovered.required").write_text("", encoding="utf-8")
            (root / "migration" / "coverage-uncovered.allowlist").write_text(
                json.dumps({
                    "file": "packages/example/core.lua",
                    "line": 2,
                    "normalized_line_hash": "abcdef12",
                    "reason": "legacy uncovered branch",
                }) + "\n",
                encoding="utf-8",
            )
            artifact = root / "coverage.json"
            artifact.write_text(
                json.dumps({
                    "files": [{
                        "file": "packages/example/core.lua",
                        "missing_lines": [{
                            "line": 2,
                            "normalized_line_hash": "abcdef12",
                            "text": "return missing_branch()",
                        }],
                    }],
                }),
                encoding="utf-8",
            )

            with mock.patch.dict("os.environ", {"FKST_LUA_COVERAGE_JSON": str(artifact)}, clear=False):
                with mock.patch.object(coverage, "selected_base_ref", return_value="integration"):
                    with mock.patch.object(coverage, "required_flag_at_base", return_value="absent"):
                        with mock.patch.object(coverage, "allowlist_at_base", return_value=("absent", None)):
                            messages = coverage.repository_messages(root)

        self.assertEqual(messages, [])

    def test_repository_messages_tolerates_rollup_line_shift_by_content(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "migration").mkdir()
            (root / "migration" / "coverage-uncovered.required").write_text("", encoding="utf-8")
            # Rollup PR #1208 failed when the same uncovered ready.lua content
            # moved from line 71 to line 74 while the allowlist stayed line-keyed.
            (root / "migration" / "coverage-uncovered.allowlist").write_text(
                json.dumps({
                    "file": "packages/github-devloop/core/restart/transitions/ready.lua",
                    "line": 71,
                    "normalized_line_hash": "af87eb9432e4a024",
                    "reason": "baseline",
                }) + "\n",
                encoding="utf-8",
            )
            artifact = root / "coverage.json"
            artifact.write_text(
                json.dumps({
                    "files": [{
                        "file": "packages/github-devloop/core/restart/transitions/ready.lua",
                        "missing_lines": [{
                            "line": 74,
                            "normalized_line_hash": "af87eb9432e4a024",
                            "text": "\"result_effects_complete\"",
                        }],
                    }],
                }),
                encoding="utf-8",
            )
            shifted_key = coverage.CoverageKey(
                "packages/github-devloop/core/restart/transitions/ready.lua",
                74,
                "af87eb9432e4a024",
            )
            old_line_keyed_allowlist = {
                coverage.CoverageKey(
                    "packages/github-devloop/core/restart/transitions/ready.lua",
                    71,
                    "af87eb9432e4a024",
                )
            }
            old_line_keyed_uncovered = {
                shifted_key: coverage.UncoveredLine(shifted_key, '"result_effects_complete"')
            }
            old_line_keyed_messages = [
                f"{old_line_keyed_uncovered[key].label()} "
                "is an uncovered production Lua line not in migration/coverage-uncovered.allowlist"
                for key in sorted(set(old_line_keyed_uncovered) - old_line_keyed_allowlist)
            ]
            self.assertEqual(len(old_line_keyed_messages), 1)
            self.assertIn("not in migration/coverage-uncovered.allowlist", old_line_keyed_messages[0])

            with mock.patch.dict("os.environ", {"FKST_LUA_COVERAGE_JSON": str(artifact)}, clear=False):
                with mock.patch.object(coverage, "selected_base_ref", return_value="integration"):
                    with mock.patch.object(coverage, "required_flag_at_base", return_value="absent"):
                        messages = coverage.repository_messages(root)

        self.assertEqual(messages, [])

    def test_repository_messages_warns_for_stale_allowlist_without_blocking(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "migration").mkdir()
            (root / "migration" / "coverage-uncovered.required").write_text("", encoding="utf-8")
            (root / "migration" / "coverage-uncovered.allowlist").write_text(
                json.dumps({
                    "file": "packages/example/core.lua",
                    "line": 2,
                    "normalized_line_hash": "abcdef12",
                    "reason": "legacy uncovered branch",
                }) + "\n",
                encoding="utf-8",
            )
            artifact = root / "coverage.json"
            artifact.write_text(json.dumps({"files": []}), encoding="utf-8")

            with mock.patch.dict("os.environ", {"FKST_LUA_COVERAGE_JSON": str(artifact)}, clear=False):
                with mock.patch.object(coverage, "selected_base_ref", return_value="integration"):
                    with mock.patch.object(coverage, "required_flag_at_base", return_value="absent"):
                        with mock.patch.object(coverage, "allowlist_at_base", return_value=("absent", None)):
                            with mock.patch("sys.stderr") as stderr:
                                messages = coverage.repository_messages(root)

        self.assertEqual(messages, [])
        self.assertIn(
            "is no longer uncovered; prune the stale entry",
            "".join(call.args[0] for call in stderr.write.call_args_list),
        )

    def test_repository_messages_uses_selected_base_ref(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "migration").mkdir()
            (root / "migration" / "coverage-uncovered.required").write_text("", encoding="utf-8")
            (root / "migration" / "coverage-uncovered.allowlist").write_text(
                json.dumps({
                    "file": "packages/example/core.lua",
                    "line": 2,
                    "normalized_line_hash": "abcdef12",
                    "reason": "legacy uncovered branch",
                }) + "\n",
                encoding="utf-8",
            )
            artifact = root / "coverage.json"
            artifact.write_text(
                json.dumps({
                    "files": [{
                        "file": "packages/example/core.lua",
                        "missing_lines": [{
                            "line": 2,
                            "normalized_line_hash": "abcdef12",
                            "text": "return missing_branch()",
                        }],
                    }],
                }),
                encoding="utf-8",
            )

            with mock.patch.dict("os.environ", {"FKST_LUA_COVERAGE_JSON": str(artifact)}, clear=False):
                with mock.patch.object(coverage, "selected_base_ref", return_value="origin/integration"):
                    with mock.patch.object(coverage, "required_flag_at_base", return_value="present"):
                        with mock.patch.object(coverage, "allowlist_at_base", return_value=("present", set())) as base:
                            messages = coverage.repository_messages(root)

        base.assert_called_once_with(root, "origin/integration")
        self.assertIn("relative to origin/integration", messages[-1])

    def test_repository_messages_requires_configured_base_ref(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "migration").mkdir()
            (root / "migration" / "coverage-uncovered.required").write_text("", encoding="utf-8")
            artifact = root / "coverage.json"
            artifact.write_text(
                json.dumps({
                    "files": [{
                        "file": "packages/example/core.lua",
                        "missing_lines": [{
                            "line": 2,
                            "normalized_line_hash": "abcdef12",
                            "text": "return missing_branch()",
                        }],
                    }],
                }),
                encoding="utf-8",
            )

            with mock.patch.dict("os.environ", {"FKST_LUA_COVERAGE_JSON": str(artifact)}, clear=False):
                with mock.patch.object(coverage, "selected_base_ref", return_value=None):
                    messages = coverage.repository_messages(root)

        self.assertIn("cannot resolve coverage base allowlist", messages[0])

    def test_repository_messages_first_activation_skips_base_growth_check(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "migration").mkdir()
            (root / "migration" / "coverage-uncovered.required").write_text("", encoding="utf-8")
            (root / "migration" / "coverage-uncovered.allowlist").write_text(
                json.dumps({
                    "file": "packages/example/core.lua",
                    "line": 2,
                    "normalized_line_hash": "abcdef12",
                    "reason": "baseline",
                }) + "\n",
                encoding="utf-8",
            )
            artifact = root / "coverage.json"
            artifact.write_text(
                json.dumps({
                    "files": [{
                        "file": "packages/example/core.lua",
                        "missing_lines": [{
                            "line": 2,
                            "normalized_line_hash": "abcdef12",
                            "text": "return missing_branch()",
                        }],
                    }],
                }),
                encoding="utf-8",
            )

            with mock.patch.dict("os.environ", {"FKST_LUA_COVERAGE_JSON": str(artifact)}, clear=False):
                with mock.patch.object(coverage, "selected_base_ref", return_value="integration"):
                    with mock.patch.object(coverage, "required_flag_at_base", return_value="absent"):
                        messages = coverage.repository_messages(root)

        self.assertEqual(messages, [])

    def test_repository_messages_report_only_without_required_flag(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            artifact = root / "coverage.json"
            artifact.write_text(
                json.dumps({
                    "files": [{
                        "file": "packages/example/core.lua",
                        "missing_lines": [{
                            "line": 2,
                            "normalized_line_hash": "abcdef12",
                            "text": "return missing_branch()",
                        }],
                    }],
                }),
                encoding="utf-8",
            )

            with mock.patch.dict("os.environ", {"FKST_LUA_COVERAGE_JSON": str(artifact)}, clear=False):
                with mock.patch.object(coverage, "selected_base_ref", return_value=None):
                    with mock.patch("sys.stderr") as stderr:
                        messages = coverage.repository_messages(root)

        self.assertEqual(messages, [])
        self.assertIn("1 uncovered line(s) would block once enabled", "".join(call.args[0] for call in stderr.write.call_args_list))

    def test_repository_messages_reports_empty_canonical_artifact_as_missing_line_metadata(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            artifact = root / "coverage.json"
            artifact.write_text("{}", encoding="utf-8")

            with mock.patch.dict("os.environ", {"FKST_LUA_COVERAGE_JSON": str(artifact)}, clear=False):
                with mock.patch.object(coverage, "selected_base_ref", return_value=None):
                    with mock.patch("sys.stderr") as stderr:
                        messages = coverage.repository_messages(root)

        self.assertEqual(messages, [])
        self.assertIn(
            "coverage artifact would not parse once enabled: coverage artifact has no covered-line metadata",
            "".join(call.args[0] for call in stderr.write.call_args_list),
        )

    def test_repository_messages_required_flag_enables_blocking(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "migration").mkdir()
            (root / "migration" / "coverage-uncovered.required").write_text("", encoding="utf-8")
            artifact = root / "coverage.json"
            artifact.write_text(
                json.dumps({
                    "files": [{
                        "file": "packages/example/core.lua",
                        "missing_lines": [{
                            "line": 2,
                            "normalized_line_hash": "abcdef12",
                            "text": "return missing_branch()",
                        }],
                    }],
                }),
                encoding="utf-8",
            )

            with mock.patch.dict("os.environ", {"FKST_LUA_COVERAGE_JSON": str(artifact)}, clear=False):
                with mock.patch.object(coverage, "selected_base_ref", return_value="integration"):
                    with mock.patch.object(coverage, "required_flag_at_base", return_value="absent"):
                        with mock.patch.object(coverage, "allowlist_at_base", return_value=("absent", None)):
                            messages = coverage.repository_messages(root)

        self.assertEqual(len(messages), 1)
        self.assertIn("not in migration/coverage-uncovered.allowlist", messages[0])

    def test_repository_messages_required_flag_without_artifact_defers(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "migration").mkdir()
            (root / "migration" / "coverage-uncovered.required").write_text("", encoding="utf-8")

            with mock.patch.dict("os.environ", {}, clear=True):
                with mock.patch.object(coverage, "selected_base_ref", return_value=None):
                    with mock.patch("sys.stderr") as stderr:
                        messages = coverage.repository_messages(root)

        self.assertEqual(messages, [])
        warning = "".join(call.args[0] for call in stderr.write.call_args_list)
        self.assertIn("Lua coverage ratchet deferred", warning)
        self.assertIn("no coverage artifact is available", warning)

    def test_repository_messages_required_flag_explicit_missing_artifact_fails(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "migration").mkdir()
            (root / "migration" / "coverage-uncovered.required").write_text("", encoding="utf-8")
            artifact = root / "missing-coverage.json"

            with mock.patch.dict("os.environ", {"FKST_LUA_COVERAGE_JSON": str(artifact)}, clear=True):
                with mock.patch.object(coverage, "selected_base_ref", return_value=None):
                    messages = coverage.repository_messages(root)

        self.assertEqual(messages, [f"Lua coverage artifact does not exist: {artifact}"])

    def test_repository_messages_ignores_coverage_json_env_without_required_flag(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            artifact = root / "coverage.json"
            artifact.write_text(
                json.dumps({
                    "files": [{
                        "file": "packages/example/core.lua",
                        "missing_lines": [{
                            "line": 2,
                            "normalized_line_hash": "abcdef12",
                            "text": "return missing_branch()",
                        }],
                    }],
                }),
                encoding="utf-8",
            )

            with mock.patch.dict("os.environ", {"FKST_LUA_COVERAGE_JSON": str(artifact)}, clear=False):
                with mock.patch.object(coverage, "selected_base_ref", return_value=None):
                    messages = coverage.repository_messages(root)

        self.assertEqual(messages, [])

    def test_repository_messages_advisory_when_required_flag_removed(self) -> None:
        # Coverage is advisory: with no REQUIRED_FLAG, uncovered lines are reported
        # as a warning, not a blocking message, even when base still has the flag.
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            artifact = root / "coverage.json"
            artifact.write_text(
                json.dumps({
                    "files": [{
                        "file": "packages/example/core.lua",
                        "missing_lines": [{
                            "line": 2,
                            "normalized_line_hash": "abcdef12",
                            "text": "return missing_branch()",
                        }],
                    }],
                }),
                encoding="utf-8",
            )

            with mock.patch.dict("os.environ", {"FKST_LUA_COVERAGE_JSON": str(artifact)}, clear=False):
                with mock.patch.object(coverage, "selected_base_ref", return_value="integration"):
                    with mock.patch.object(coverage, "required_flag_at_base", return_value="present"):
                        with mock.patch("sys.stderr"):
                            messages = coverage.repository_messages(root)

        self.assertEqual(messages, [])

    def test_allowlist_at_base_works_from_linked_worktree(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp) / "repo"
            linked = Path(tmp) / "linked"
            root.mkdir()
            git(root, "init")
            git(root, "config", "user.email", "fkst-test@example.invalid")
            git(root, "config", "user.name", "fkst test")

            (root / "migration").mkdir()
            (root / "migration" / "coverage-uncovered.allowlist").write_text(
                json.dumps({
                    "file": "packages/example/core.lua",
                    "line": 2,
                    "normalized_line_hash": "abcdef12",
                    "reason": "baseline",
                }) + "\n",
                encoding="utf-8",
            )
            git(root, "add", "migration/coverage-uncovered.allowlist")
            git(root, "commit", "-m", "seed coverage allowlist")
            base_commit = git(root, "rev-parse", "HEAD")
            git(root, "worktree", "add", str(linked), "HEAD")

            status, entries = coverage.allowlist_at_base(linked, base_commit)

        self.assertEqual(status, "present")
        self.assertEqual(entries, {self.key()})


if __name__ == "__main__":
    unittest.main()
