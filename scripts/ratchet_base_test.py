#!/usr/bin/env python3
"""Unit tests for the shared dev-base ratchet resolver."""

from __future__ import annotations

import os
import subprocess
import tempfile
import unittest
from pathlib import Path
from unittest import mock

import ratchet_base


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


def init_repo(root: Path) -> None:
    git(root, "init")
    git(root, "config", "user.email", "fkst-test@example.invalid")
    git(root, "config", "user.name", "fkst test")


def commit_file(root: Path, path: str, text: str, message: str) -> str:
    full = root / path
    full.parent.mkdir(parents=True, exist_ok=True)
    full.write_text(text, encoding="utf-8")
    git(root, "add", path)
    git(root, "commit", "-m", message)
    return git(root, "rev-parse", "HEAD")


class RatchetBaseTest(unittest.TestCase):
    def test_env_override_wins_over_origin_dev(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            init_repo(root)
            origin_commit = commit_file(root, "base.txt", "origin\n", "origin")
            git(root, "update-ref", "refs/remotes/origin/dev", origin_commit)
            override_commit = commit_file(root, "base.txt", "override\n", "override")
            git(root, "tag", "custom-dev", override_commit)

            with mock.patch.dict(os.environ, {"FKST_RATCHET_DEV_REF": "custom-dev"}):
                self.assertEqual(ratchet_base.resolve_dev_ref(root), override_commit)

    def test_falls_through_to_origin_dev_without_local_dev(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            init_repo(root)
            origin_commit = commit_file(root, "base.txt", "origin\n", "origin")
            git(root, "update-ref", "refs/remotes/origin/dev", origin_commit)
            head_commit = commit_file(root, "base.txt", "head\n", "head")

            self.assertEqual(ratchet_base.resolve_dev_ref(root), origin_commit)
            self.assertEqual(ratchet_base.resolve_dev_merge_base(root), origin_commit)
            self.assertNotEqual(origin_commit, head_commit)

    def test_origin_dev_wins_over_different_local_dev(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            init_repo(root)
            origin_commit = commit_file(root, "base.txt", "origin\n", "origin")
            git(root, "update-ref", "refs/remotes/origin/dev", origin_commit)
            local_commit = commit_file(root, "base.txt", "local\n", "local")
            git(root, "update-ref", "refs/heads/dev", local_commit)

            self.assertEqual(ratchet_base.resolve_dev_ref(root), origin_commit)
            self.assertEqual(ratchet_base.resolve_dev_merge_base(root), origin_commit)
            self.assertNotEqual(origin_commit, local_commit)

    def test_show_file_at_present_and_absent(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            init_repo(root)
            commit = commit_file(root, "migration/example.allowlist", "one\n# two\n", "base")

            self.assertEqual(ratchet_base.show_file_at(root, commit, "migration/example.allowlist"), "one\n# two\n")
            self.assertIsNone(ratchet_base.show_file_at(root, commit, "migration/missing.allowlist"))

    def test_file_at_base_present_absent_and_unresolved_ref(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            init_repo(root)
            origin_commit = commit_file(root, "migration/example.allowlist", "one\n# two\n", "base")
            git(root, "update-ref", "refs/remotes/origin/dev", origin_commit)
            commit_file(root, "migration/example.allowlist", "head\n", "head")

            self.assertEqual(ratchet_base.file_at_base(root, "migration/example.allowlist"), ("present", "one\n# two\n"))
            self.assertEqual(ratchet_base.file_at_base(root, "migration/missing.allowlist"), ("absent", None))

        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            init_repo(root)
            commit_file(root, "migration/example.allowlist", "one\n", "base")

            self.assertEqual(ratchet_base.file_at_base(root, "migration/example.allowlist"), ("unresolved", None))

    def test_file_at_base_show_failure_after_existence_is_unresolved(self) -> None:
        ok_exists = subprocess.CompletedProcess(["git"], 0, "", "")
        failed_show = subprocess.CompletedProcess(["git"], 1, "", "")
        with mock.patch.object(ratchet_base, "resolve_dev_merge_base", return_value="abc123"), \
                mock.patch.object(ratchet_base, "_git", side_effect=(ok_exists, failed_show)):
            status, text = ratchet_base.file_at_base(Path("/unused"), "migration/example.allowlist")

        self.assertEqual((status, text), ("unresolved", None))


if __name__ == "__main__":
    unittest.main()
