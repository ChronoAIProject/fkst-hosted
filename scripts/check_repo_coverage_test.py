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

    def test_stale_allowlist_entry_forces_prune(self) -> None:
        messages = coverage.ratchet_messages({}, {self.key()})

        self.assertEqual(len(messages), 1)
        self.assertIn("is no longer uncovered; prune the stale entry", messages[0])

    def test_allowlist_growth_relative_to_base_fails(self) -> None:
        old_key = self.key(line=1, digest="11111111")
        current = {old_key, self.key()}
        base = {old_key}

        messages = coverage.ratchet_messages({}, current, base, "integration")

        self.assertIn("grows migration/coverage-uncovered.allowlist relative to integration", messages[-1])

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
                            "file": "std/zeta.lua",
                            "missing_lines": [{
                                "line": 7,
                                "normalized_line_hash": "bbbbbbbb",
                                "text": "return zeta()",
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

        self.assertEqual(count, 2)
        self.assertEqual(count_again, 2)
        self.assertEqual(first, second)
        self.assertEqual(
            [json.loads(line) for line in first.splitlines()],
            [
                {
                    "file": "packages/example/core.lua",
                    "line": 2,
                    "normalized_line_hash": "abcdef12",
                    "reason": "baseline",
                },
                {
                    "file": "std/zeta.lua",
                    "line": 7,
                    "normalized_line_hash": "bbbbbbbb",
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
                "line": 5,
                "normalized_line_hash": coverage.normalized_source_hash("function M.missing()"),
                "reason": "baseline",
            },
            entries,
        )
        self.assertIn(
            {
                "file": "packages/example/unused.lua",
                "line": 2,
                "normalized_line_hash": coverage.normalized_source_hash("function M.unused()"),
                "reason": "baseline",
            },
            entries,
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
                    with mock.patch.object(coverage, "allowlist_at_base", return_value=("absent", None)):
                        messages = coverage.repository_messages(root)

        self.assertEqual(messages, [])

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
                    with mock.patch.object(coverage, "allowlist_at_base", return_value=("absent", None)):
                        messages = coverage.repository_messages(root)

        self.assertEqual(len(messages), 1)
        self.assertIn("not in migration/coverage-uncovered.allowlist", messages[0])

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

    def test_repository_messages_blocks_required_flag_removal(self) -> None:
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
                        messages = coverage.repository_messages(root)

        self.assertEqual(messages, [
            "migration/coverage-uncovered.required may not be removed; coverage ratchet is enabled on base"
        ])

    def test_repository_messages_blocks_required_flag_removal_via_real_base_lookup(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            git(root, "init")
            git(root, "config", "user.email", "fkst-test@example.invalid")
            git(root, "config", "user.name", "fkst test")

            (root / "migration").mkdir()
            (root / "migration" / "coverage-uncovered.required").write_text("", encoding="utf-8")
            git(root, "add", "migration/coverage-uncovered.required")
            git(root, "commit", "-m", "enable coverage ratchet")
            base_commit = git(root, "rev-parse", "HEAD")

            (root / "migration" / "coverage-uncovered.required").unlink()
            git(root, "add", "-u")
            git(root, "commit", "-m", "remove coverage ratchet flag")

            with mock.patch.dict("os.environ", {"FKST_LUA_COVERAGE_BASE_REF": base_commit}, clear=False):
                messages = coverage.repository_messages(root)

        self.assertEqual(messages, [
            "migration/coverage-uncovered.required may not be removed; coverage ratchet is enabled on base"
        ])


if __name__ == "__main__":
    unittest.main()
