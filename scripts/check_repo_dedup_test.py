#!/usr/bin/env python3
"""Unit tests for the G-DEDUP production Lua duplicate ratchet."""

from __future__ import annotations

import importlib.util
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


scripts_dir = Path(__file__).resolve().parent
check_repo = load_module("check_repo", scripts_dir / "check_repo.py")
dedup = check_repo.check_repo_dedup


def git(root: Path, *args: str) -> str:
    result = subprocess.run(
        ["git", *args],
        cwd=root,
        check=True,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )
    return result.stdout.strip()


def init_git_repo(root: Path) -> None:
    git(root, "init")
    git(root, "config", "user.email", "fkst-test@example.invalid")
    git(root, "config", "user.name", "fkst test")


def commit_paths(root: Path, paths: list[str], message: str) -> str:
    git(root, "add", *paths)
    git(root, "commit", "-m", message)
    return git(root, "rev-parse", "HEAD")


def duplicate_body() -> str:
    return (
        "  local total = 0\n"
        "  for index, item in ipairs(items) do\n"
        "    total = total + index + item.amount\n"
        "  end\n"
        "  return total\n"
    )


class DedupRatchetTest(unittest.TestCase):
    def sources(self, body: str | None = None) -> dict[str, str]:
        body = duplicate_body() if body is None else body
        return {
            "packages/one/core.lua": f"local function shared_total(items)\n{body}end\n",
            "packages/two/core.lua": f"function M.shared_total(items)\n{body}end\n",
        }

    def test_new_cross_file_duplicate_production_function_fails(self) -> None:
        messages = dedup.ratchet_messages(self.sources(), allowlist=set())

        self.assertEqual(len(messages), 1)
        self.assertIn("shared_total", messages[0])
        self.assertIn("not in the allowlist baseline", messages[0])

    def test_allowlisted_duplicate_passes(self) -> None:
        groups = dedup.duplicate_groups(self.sources())
        allowlist = {next(iter(groups))}

        self.assertEqual(dedup.ratchet_messages(self.sources(), allowlist), [])

    def test_nested_function_definitions_are_not_module_scope_units(self) -> None:
        sources = {
            "packages/one/core.lua": (
                "local function top_one(items)\n"
                "local function ignored(items)\n"
                "  return items\n"
                "end\n"
                "local function shared_total(items)\n"
                f"{duplicate_body()}"
                "end\n"
                "return shared_total(items) + 1\n"
                "end\n"
            ),
            "packages/two/core.lua": (
                "local function top_two(items)\n"
                "local function ignored(items)\n"
                "  return items\n"
                "end\n"
                "local function shared_total(items)\n"
                f"{duplicate_body()}"
                "end\n"
                "return shared_total(items) + 2\n"
                "end\n"
            ),
        }

        self.assertEqual(dedup.duplicate_groups(sources), set())

    def test_stale_allowlist_entry_forces_prune(self) -> None:
        stale = dedup.DedupEntry(
            name="old_helper",
            body_hash="0123456789abcdef0123456789abcdef",
            files=("packages/one/core.lua", "packages/two/core.lua"),
        )

        messages = dedup.ratchet_messages({}, {stale})

        self.assertEqual(len(messages), 1)
        self.assertIn("no longer matches a duplicate group", messages[0])
        self.assertIn("prune", messages[0])

    def test_allowlist_growth_relative_to_base_fails(self) -> None:
        groups = dedup.duplicate_groups(self.sources())
        current = set(groups)
        base: set[object] = set()

        messages = dedup.ratchet_messages(self.sources(), current, base)

        self.assertEqual(len(messages), 1)
        self.assertIn("grows code-dedup allowlist relative to dev", messages[0])

    def test_partial_duplicate_group_shrink_does_not_grow_allowlist(self) -> None:
        current = next(iter(dedup.duplicate_groups(self.sources())))
        base = dedup.DedupEntry(
            name=current.name,
            body_hash=current.body_hash,
            files=tuple(sorted((*current.files, "packages/three/core.lua"))),
        )

        messages = dedup.ratchet_messages(self.sources(), {current}, {base})

        self.assertEqual(messages, [])

    def test_dev_base_allowlist_resolves_from_origin_dev_without_local_dev(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            init_git_repo(root)
            (root / "migration").mkdir()
            entry = dedup.DedupEntry(
                name="shared_total",
                body_hash="0123456789abcdef0123456789abcdef",
                files=("packages/one/core.lua", "packages/two/core.lua"),
            )
            (root / dedup.ALLOWLIST).write_text(f"# comment\n{entry.allowlist_line()}\n\n", encoding="utf-8")
            base_commit = commit_paths(root, [dedup.ALLOWLIST], "base allowlist")
            git(root, "update-ref", "refs/remotes/origin/dev", base_commit)
            (root / dedup.ALLOWLIST).write_text("", encoding="utf-8")
            commit_paths(root, [dedup.ALLOWLIST], "head allowlist")

            status, allowlist = dedup.allowlist_at_dev_base(root)

        self.assertEqual(status, "present")
        self.assertEqual(allowlist, {entry})

    def test_production_sources_exclude_tests_helpers_and_fakes(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            for relpath in (
                "packages/one/core.lua",
                "packages/one/tests/core_test.lua",
                "packages/two/helper_helpers.lua",
                "packages/two/github_fake.lua",
                "libraries/forge/example.lua",
            ):
                path = root / relpath
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text("return {}\n", encoding="utf-8")

            sources = dedup.sources(root, root / "packages", check_repo.read_text, check_repo.rel)

        self.assertEqual(set(sources), {"packages/one/core.lua", "libraries/forge/example.lua"})

    def test_check_repo_wrapper_loads_allowlist_and_prefixes_violations(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "migration").mkdir()
            for relpath in ("packages/one/core.lua", "packages/two/core.lua"):
                path = root / relpath
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text(
                    f"local function shared_total(items)\n{duplicate_body()}end\n",
                    encoding="utf-8",
                )

            groups = dedup.duplicate_groups(dedup.sources(root, root / "packages", check_repo.read_text, check_repo.rel))
            (root / dedup.ALLOWLIST).write_text(
                "\n".join(entry.allowlist_line() for entry in groups) + "\n",
                encoding="utf-8",
            )

            violations: list[str] = []
            with mock.patch.object(check_repo.check_repo_dedup, "allowlist_at_dev_base", return_value=("present", set(groups))):
                check_repo.check_code_dedup_ratchet(root, violations)

            self.assertEqual(violations, [])

            (root / "packages" / "two" / "core.lua").write_text("return {}\n", encoding="utf-8")
            with mock.patch.object(check_repo.check_repo_dedup, "allowlist_at_dev_base", return_value=("present", set(groups))):
                check_repo.check_code_dedup_ratchet(root, violations)

        self.assertEqual(len(violations), 1)
        self.assertTrue(violations[0].startswith("G-DEDUP: "))
        self.assertIn("no longer matches a duplicate group", violations[0])


if __name__ == "__main__":
    unittest.main()
