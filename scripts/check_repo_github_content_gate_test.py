#!/usr/bin/env python3
"""Tests for the codex-bundle GitHub content provenance guard."""

from __future__ import annotations

import importlib.util
import sys
import tempfile
import unittest
from pathlib import Path


def load_check_repo():
    path = Path(__file__).with_name("check_repo.py")
    scripts_dir = str(path.parent)
    if scripts_dir not in sys.path:
        sys.path.insert(0, scripts_dir)
    spec = importlib.util.spec_from_file_location("check_repo", path)
    if spec is None or spec.loader is None:
        raise RuntimeError("could not load check_repo.py")
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


check_repo = load_check_repo()


GOOD_BUNDLE_SOURCE = """
local content_provenance = require("devloop.content_provenance")
issue_json = content_provenance.filter_gh_content_json(issue_json, "issue", whitelist, issue_redactions)
pr_json = content_provenance.filter_gh_content_json(pr_json, "pr", whitelist, pr_redactions)
"""


class GithubContentGateGuardTest(unittest.TestCase):
    def run_guard(
        self,
        bundle_source: str,
        *,
        shared_source: str = "",
        shared_path: str = "libraries/devloop/github_proxy_entity_view.lua",
    ) -> list[str]:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            bundle = root / "libraries" / "devloop" / "context_bundle.lua"
            bundle.parent.mkdir(parents=True)
            bundle.write_text(bundle_source, encoding="utf-8")
            shared = root / shared_path
            shared.parent.mkdir(parents=True, exist_ok=True)
            shared.write_text(shared_source, encoding="utf-8")
            violations: list[str] = []
            check_repo.check_github_content_gate(root, violations)
            return violations

    def test_current_repository_shape_passes(self) -> None:
        root = Path(__file__).resolve().parents[1]
        violations: list[str] = []
        check_repo.check_github_content_gate(root, violations)
        self.assertEqual(violations, [])

    def test_missing_issue_or_pr_bundle_filter_fails(self) -> None:
        cases = {
            "issue": GOOD_BUNDLE_SOURCE.replace(
                'issue_json = content_provenance.filter_gh_content_json(issue_json, "issue", whitelist, issue_redactions)\n',
                "",
            ),
            "pr": GOOD_BUNDLE_SOURCE.replace(
                'pr_json = content_provenance.filter_gh_content_json(pr_json, "pr", whitelist, pr_redactions)\n',
                "",
            ),
        }
        for missing, source in cases.items():
            with self.subTest(missing=missing):
                violations = self.run_guard(source)
                self.assertEqual(len(violations), 1)
                self.assertIn("G-GITHUB-CONTENT-GATE", violations[0])
                self.assertIn(f'"{missing}"', violations[0])

    def test_filtering_shared_view_files_fails(self) -> None:
        for shared_path in (
            "libraries/devloop/github_proxy_entity_view.lua",
            "packages/github-proxy/core/rest_view.lua",
        ):
            with self.subTest(shared_path=shared_path):
                violations = self.run_guard(
                    GOOD_BUNDLE_SOURCE,
                    shared_source='return require("devloop.content_provenance").filter_gh_content_json\n',
                    shared_path=shared_path,
                )
                self.assertEqual(len(violations), 1)
                self.assertIn("G-GITHUB-CONTENT-GATE", violations[0])
                self.assertIn(shared_path, violations[0])

    def test_intended_bundle_only_shape_passes(self) -> None:
        self.assertEqual(self.run_guard(GOOD_BUNDLE_SOURCE), [])


if __name__ == "__main__":
    unittest.main()
