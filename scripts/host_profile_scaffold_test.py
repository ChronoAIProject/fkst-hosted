#!/usr/bin/env python3
"""Contract tests for the host profile documentation scaffold."""

from __future__ import annotations

import re
import unittest
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[1]


class HostProfileScaffoldTest(unittest.TestCase):
    def read(self, relative: str) -> str:
        return (REPO_ROOT / relative).read_text(encoding="utf-8")

    def uncommented_assignment_names(self, content: str) -> set[str]:
        names: set[str] = set()
        for raw in content.splitlines():
            line = raw.strip()
            if not line or line.startswith("#"):
                continue
            match = re.match(r"^([A-Za-z_][A-Za-z0-9_]*)=", line)
            if match:
                names.add(match.group(1))
        return names

    def test_global_host_profile_doc_pins_xdg_location_and_explicit_invocation(self) -> None:
        doc = self.read("docs/user/global-host-profiles.md")

        self.assertIn("${XDG_CONFIG_HOME:-$HOME/.config}/fkst/host.env", doc)
        self.assertIn('scripts/run.sh host --host-root "$FKST_HOST_ROOT"', doc)
        self.assertIn("fkst.workspace.toml", doc)
        self.assertIn("fkst.lock", doc)
        self.assertIn("There is no `--profile <name>`", doc)
        self.assertIn("Documentation beats scaffolds; explicit CLI/env beats documentation.", doc)
        self.assertNotIn("AI:FKST", doc)

    def test_host_profile_scaffold_exports_existing_host_run_facts_only(self) -> None:
        scaffold = self.read("docs/user/host-profile.env.example")
        assignments = self.uncommented_assignment_names(scaffold)

        self.assertTrue(
            {
                "BIN",
                "FKST_HOST_ROOT",
                "FKST_PLATFORM_ROOT",
                "FKST_DURABLE_ROOT",
                "FKST_RATE_POOL_ROOT",
                "FKST_GITHUB_REPO",
                "FKST_GITHUB_BOT_LOGIN",
                "FKST_DEVLOOP_INTEGRATION_BRANCH",
            }.issubset(assignments)
        )
        self.assertNotIn("FKST_PROFILE", assignments)
        self.assertNotIn("FKST_PROFILE_NAME", assignments)
        self.assertNotRegex(scaffold, r"(?m)^FKST_RUNTIME_ROOT=")
        self.assertNotIn("chmod", scaffold)

    def test_documentation_index_links_global_host_profiles(self) -> None:
        docs_index = self.read("docs/README.md")
        readme = self.read("README.md")

        self.assertIn("user/global-host-profiles.md", docs_index)
        self.assertIn("docs/user/global-host-profiles.md", readme)


if __name__ == "__main__":
    unittest.main()
