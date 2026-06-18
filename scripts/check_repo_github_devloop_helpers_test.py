#!/usr/bin/env python3
"""Unit tests for github-devloop helper ownership guards."""

from __future__ import annotations

import importlib.util
import sys
import unittest
from pathlib import Path


def load_module(name: str, path: Path):
    spec = importlib.util.spec_from_file_location(name, path)
    if spec is None or spec.loader is None:
        raise RuntimeError(f"could not load {path.name}")
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


scripts_dir = Path(__file__).resolve().parent
check_repo = load_module("check_repo", scripts_dir / "check_repo.py")
guard = check_repo.check_repo_github_devloop_helpers


class GithubDevloopNameOnlyPathHelperGuardTest(unittest.TestCase):
    def messages(self, sources: dict[str, str]) -> list[str]:
        return guard.messages(sources, check_repo.strip_lua_comments_and_strings)

    def test_allows_core_export_with_recurrence_api_waiver(self) -> None:
        sources = {
            "packages/github-devloop/core.lua": """
-- Recurrence/API waiver: `parse_name_only_paths` is package-root API because the
-- repository guard forbids local copies of this package-local helper.
function M.parse_name_only_paths(stdout)
  return {}
end
""",
            "packages/github-devloop/core/queue.lua": """
local core = require("core")
return core.parse_name_only_paths(stdout)
""",
        }

        self.assertEqual(self.messages(sources), [])

    def test_rejects_local_name_only_path_helper_copy(self) -> None:
        sources = {
            "packages/github-devloop/core.lua": """
-- Recurrence/API waiver: `parse_name_only_paths` is package-root API because the
-- repository guard forbids local copies of this package-local helper.
function M.parse_name_only_paths(stdout)
  return {}
end
""",
            "packages/github-devloop/core/queue.lua": """
local function parse_name_only_paths(stdout)
  return {}
end
""",
        }

        messages = self.messages(sources)

        self.assertEqual(len(messages), 1)
        self.assertIn("local parse_name_only_paths helper", messages[0])

    def test_rejects_core_export_without_recurrence_api_waiver(self) -> None:
        sources = {
            "packages/github-devloop/core.lua": """
function M.parse_name_only_paths(stdout)
  return {}
end
""",
        }

        messages = self.messages(sources)

        self.assertEqual(len(messages), 1)
        self.assertIn("must carry an explicit Recurrence/API waiver", messages[0])


if __name__ == "__main__":
    unittest.main()
