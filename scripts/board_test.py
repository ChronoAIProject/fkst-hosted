#!/usr/bin/env python3
"""Behavior tests for scripts/run.sh board."""

from __future__ import annotations

import json
import os
import stat
import subprocess
import tempfile
import textwrap
import unittest
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[1]


def write_executable(path: Path, text: str) -> None:
    path.write_text(text, encoding="utf-8")
    path.chmod(path.stat().st_mode | stat.S_IXUSR)


class BoardHarness:
    def __init__(self, observe: dict | None = None, exit_code: int = 0, stderr: str = "") -> None:
        self.tmp = tempfile.TemporaryDirectory()
        self.root = Path(self.tmp.name)
        self.cache = self.root / "board-cache.json"
        self.durable = self.root / "durable"
        self.durable.mkdir()
        self.observe_path = self.root / "observe.json"
        self.log = self.root / "calls.log"
        if observe is not None:
            self.observe_path.write_text(json.dumps(observe), encoding="utf-8")
        self.framework = self.root / "fkst-framework"
        if exit_code == 0:
            body = f"cat {self.observe_path}\n"
        else:
            body = f"printf '%s\\n' {json.dumps(stderr)} >&2\nexit {exit_code}\n"
        write_executable(
            self.framework,
            textwrap.dedent(
                f"""\
#!/bin/sh
printf '%s\\n' "$*" >> {self.log}
if [ "$1" = "--self-test" ]; then
  exit 0
fi
{body}
"""
            ),
        )

    def close(self) -> None:
        self.tmp.cleanup()

    def run_board(self, *extra: str) -> subprocess.CompletedProcess[str]:
        env = os.environ.copy()
        env["BIN"] = str(self.framework)
        env["FKST_NO_AUTOBUILD"] = "1"
        return subprocess.run(
            [
                "/bin/bash",
                "scripts/run.sh",
                "board",
                "--cache",
                str(self.cache),
                "--durable-root",
                str(self.durable),
                "--now",
                "2026-06-14T10:00:00Z",
                *extra,
            ],
            cwd=REPO_ROOT,
            env=env,
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            check=False,
        )

    def calls(self) -> str:
        return self.log.read_text(encoding="utf-8") if self.log.exists() else ""


class BoardScriptTest(unittest.TestCase):
    def test_refresh_fetches_observe_json_writes_cache_and_renders_stalls(self) -> None:
        h = BoardHarness(
            {
                "entities": [
                    {
                        "entity": "github-devloop/issue/owner/repo/597",
                        "events": [
                            {"queue": "consensus.consensus_converge", "ts": "2026-06-14T09:00:00Z"},
                            {"queue": "devloop_reconcile", "ts": "2026-06-14T09:20:00Z"},
                        ],
                    },
                    {
                        "entity": "github-devloop/issue/owner/repo/598",
                        "events": [{"queue": "devloop_ready", "ts": "2026-06-14T09:59:30Z"}],
                    },
                ],
                "queues": [{"queue": "devloop_ready", "ready": 2, "leased": 1, "retry": 0, "dlq": 0}],
                "dlq": [{"queue": "devloop_ready"}],
            }
        )
        try:
            result = h.run_board("--refresh", "--stall", "1800")
            self.assertEqual(result.returncode, 0, result.stderr + result.stdout)
            self.assertIn("source=observe", result.stdout)
            self.assertIn("github-devloop/issue/owner/repo/597 latest=devloop_reconcile", result.stdout)
            self.assertIn("dwell=40m0s", result.stdout)
            self.assertIn("Stall suspects threshold=30m0s", result.stdout)
            self.assertIn("- github-devloop/issue/owner/repo/597 latest=devloop_reconcile dwell=40m0s", result.stdout)
            self.assertIn("- devloop_ready ready=2 leased=1 retry=0 dlq=0", result.stdout)
            self.assertIn("DLQ total=1", result.stdout)
            self.assertTrue(h.cache.exists())
            self.assertIn("observe --project-root", h.calls())
            self.assertIn(f"--durable-root {h.durable} --json", h.calls())
        finally:
            h.close()

    def test_fresh_cache_hit_does_not_call_engine(self) -> None:
        h = BoardHarness(exit_code=42, stderr="observe should not run")
        try:
            h.cache.write_text(
                json.dumps(
                    {
                        "schema": "fkst.board-cache.v1",
                        "cached_at": "2026-06-14T09:59:30Z",
                        "observe": {
                            "entities": [
                                {
                                    "entity": "github-devloop/issue/owner/repo/597",
                                    "events": [{"queue": "devloop_ready", "ts": "2026-06-14T09:59:00Z"}],
                                }
                            ]
                        },
                    }
                ),
                encoding="utf-8",
            )
            result = h.run_board("--ttl", "120")
            self.assertEqual(result.returncode, 0, result.stderr + result.stdout)
            self.assertIn("source=cache", result.stdout)
            self.assertIn("github-devloop/issue/owner/repo/597 latest=devloop_ready", result.stdout)
            self.assertEqual(h.calls(), "")
        finally:
            h.close()

    def test_missing_observe_command_fails_closed(self) -> None:
        h = BoardHarness(exit_code=2, stderr="unknown subcommand: observe")
        try:
            result = h.run_board("--refresh")
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("fkst-framework observe --json failed", result.stderr)
            self.assertIn("fkst-substrate#81", result.stderr)
            self.assertFalse(h.cache.exists())
        finally:
            h.close()


if __name__ == "__main__":
    unittest.main()
