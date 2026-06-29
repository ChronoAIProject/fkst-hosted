#!/usr/bin/env python3
"""Tests for the github-devloop-intake-default surface ratchet."""

from __future__ import annotations

import importlib.util
import sys
import tempfile
import textwrap
import unittest
from pathlib import Path


def load_module():
    path = Path(__file__).with_name("check_repo_intake_default_surface.py")
    spec = importlib.util.spec_from_file_location("check_repo_intake_default_surface", path)
    if spec is None or spec.loader is None:
        raise RuntimeError("could not load check_repo_intake_default_surface.py")
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


surface = load_module()


class IntakeDefaultSurfaceRatchetTest(unittest.TestCase):
    def make_repo(self) -> tuple[tempfile.TemporaryDirectory[str], Path]:
        tmp = tempfile.TemporaryDirectory()
        root = Path(tmp.name)
        (root / "libraries" / "devloop").mkdir(parents=True)
        (root / "packages" / "github-devloop-intake-default").mkdir(parents=True)
        (root / "packages" / "github-devloop-intake-default" / "core").mkdir(parents=True)
        (root / "packages" / "github-devloop-intake-default" / "tests").mkdir(parents=True)
        (root / "packages" / "some-package").mkdir(parents=True)
        (root / "libraries" / "devloop" / "github_risk.lua").write_text(
            textwrap.dedent(
                """\
                local github_risk = {}
                function github_risk.github_high_risk_path(path)
                  return tostring(path or ""):find("^scripts/") ~= nil
                end
                function github_risk.github_high_risk_paths(paths)
                  return {}
                end
                return github_risk
                """
            ),
            encoding="utf-8",
        )
        (root / "packages" / "github-devloop-intake-default" / "core.lua").write_text(
            'local M = {}\nreturn M\n',
            encoding="utf-8",
        )
        return tmp, root

    def messages(self, root: Path) -> list[str]:
        return surface.repository_messages(root)

    def assert_message_contains(self, messages: list[str], expected: str) -> None:
        self.assertTrue(
            any(expected in message for message in messages),
            f"expected message containing {expected!r}, got {messages!r}",
        )

    def test_current_single_source_shape_passes(self) -> None:
        tmp, root = self.make_repo()
        with tmp:
            self.assertEqual(self.messages(root), [])

    def test_second_high_risk_path_definition_fails(self) -> None:
        tmp, root = self.make_repo()
        with tmp:
            (root / "packages" / "github-devloop-intake-default" / "core" / "github_risk.lua").write_text(
                "function M.github_high_risk_path(path) return false end\n",
                encoding="utf-8",
            )
            messages = self.messages(root)

        self.assert_message_contains(messages, "expected exactly one typed github_high_risk_path definition")
        self.assert_message_contains(messages, "packages/github-devloop-intake-default/core/github_risk.lua")

    def test_assigned_high_risk_path_definition_fails(self) -> None:
        tmp, root = self.make_repo()
        with tmp:
            (root / "packages" / "github-devloop-intake-default" / "core" / "github_risk.lua").write_text(
                "M.github_high_risk_path = function(path) return false end\n",
                encoding="utf-8",
            )
            messages = self.messages(root)

        self.assert_message_contains(messages, "expected exactly one typed github_high_risk_path definition")
        self.assert_message_contains(messages, "packages/github-devloop-intake-default/core/github_risk.lua")

    def test_missing_canonical_high_risk_paths_definition_fails(self) -> None:
        tmp, root = self.make_repo()
        with tmp:
            (root / "libraries" / "devloop" / "github_risk.lua").write_text(
                textwrap.dedent(
                    """\
                    local github_risk = {}
                    function github_risk.github_high_risk_path(path) return false end
                    return github_risk
                    """
                ),
                encoding="utf-8",
            )
            messages = self.messages(root)

        self.assert_message_contains(messages, "expected exactly one typed github_high_risk_paths definition")

    def test_package_private_capability_export_fails(self) -> None:
        tmp, root = self.make_repo()
        with tmp:
            (root / "packages" / "github-devloop-intake-default" / "core" / "capability.lua").write_text(
                textwrap.dedent(
                    """\
                    local S = {}
                    function S.install(M)
                    function M.github_command_capability(command)
                      return { role = "read-audit" }
                    end
                    function M.github_capability_env_prefix(capability)
                      return ""
                    end
                    end
                    return S
                    """
                ),
                encoding="utf-8",
            )
            messages = self.messages(root)

        self.assert_message_contains(messages, "package-private GitHub capability export is forbidden")
        self.assert_message_contains(messages, "github_command_capability")
        self.assert_message_contains(messages, "github_capability_env_prefix")

    def test_package_private_capability_export_fails_in_other_packages(self) -> None:
        tmp, root = self.make_repo()
        with tmp:
            (root / "packages" / "some-package" / "core.lua").write_text(
                textwrap.dedent(
                    """\
                    local M = {}
                    M.github_command_capability = function(command)
                      return { role = "read-audit" }
                    end
                    return M
                    """
                ),
                encoding="utf-8",
            )
            messages = self.messages(root)

        self.assert_message_contains(messages, "package-private GitHub capability export is forbidden")
        self.assert_message_contains(messages, "packages/some-package/core.lua")

    def test_package_private_prompt_injection_canary_export_fails(self) -> None:
        tmp, root = self.make_repo()
        with tmp:
            (root / "packages" / "github-devloop-intake-default" / "core" / "canary.lua").write_text(
                textwrap.dedent(
                    """\
                    local S = {}
                    function S.install(M)
                    M.github_prompt_injection_hostile_canary = function()
                      return {}
                    end
                    end
                    return S
                    """
                ),
                encoding="utf-8",
            )
            messages = self.messages(root)

        self.assert_message_contains(messages, "package-private GitHub prompt-injection canary export is forbidden")
        self.assert_message_contains(messages, "github_prompt_injection_hostile_canary")

    def test_intake_default_core_require_of_deleted_capabilities_fails(self) -> None:
        tmp, root = self.make_repo()
        with tmp:
            (root / "packages" / "github-devloop-intake-default" / "core.lua").write_text(
                'local M = {}\nrequire("core.github_capabilities").install(M)\nreturn M\n',
                encoding="utf-8",
            )
            messages = self.messages(root)

        self.assert_message_contains(messages, "must not require core.github_capabilities")

    def test_comments_strings_and_tests_do_not_count(self) -> None:
        tmp, root = self.make_repo()
        with tmp:
            (root / "packages" / "github-devloop-intake-default" / "core" / "notes.lua").write_text(
                '-- function M.github_command_capability(command) end\nlocal s = "function M.github_high_risk_path(path)"\nreturn {}\n',
                encoding="utf-8",
            )
            (root / "packages" / "github-devloop-intake-default" / "tests" / "fixture_test.lua").write_text(
                "function M.github_prompt_injection_hostile_canary() return {} end\n",
                encoding="utf-8",
            )
            self.assertEqual(self.messages(root), [])


if __name__ == "__main__":
    unittest.main()
