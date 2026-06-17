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

    def run_health(self, *extra: str) -> subprocess.CompletedProcess[str]:
        env = os.environ.copy()
        env["BIN"] = str(self.framework)
        env["FKST_NO_AUTOBUILD"] = "1"
        return subprocess.run(
            [
                "/bin/bash",
                "scripts/run.sh",
                "health",
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

    def test_first_line_reports_healthy_when_only_expected_transients_exist(self) -> None:
        h = BoardHarness(
            {
                "entities": [
                    {
                        "entity": "github-devloop/issue/owner/repo/623",
                        "events": [
                            {
                                "queue": "devloop_ready",
                                "outcome": "retry-pending",
                                "error_class": "retry-pending",
                                "ts": "2026-06-14T09:59:30Z",
                            },
                            {
                                "queue": "github-proxy.github_entity_changed",
                                "outcome": "skip-foreign",
                                "ts": "2026-06-14T09:59:40Z",
                            },
                            {
                                "queue": "devloop_observe_tick",
                                "outcome": "deadline-defer",
                                "ts": "2026-06-14T09:00:00Z",
                            },
                            {
                                "queue": "devloop_merge_ready",
                                "error_class": "marker-lag",
                                "ts": "2026-06-14T09:00:00Z",
                            },
                        ],
                    }
                ],
                "queues": [{"queue": "devloop_ready", "ready": 0, "leased": 0, "retry": 1, "dlq": 0}],
                "failure_facts": [
                    {
                        "schema": "fkst.failure_fact.v1",
                        "origin_queue": "devloop_ready",
                        "origin_dept": "github-devloop.implement",
                        "error_class": "retry-pending",
                        "fingerprint": "retry-pending:abc",
                        "attempt": 1,
                    }
                ],
            }
        )
        try:
            result = h.run_board("--refresh", "--stall", "1800")
            self.assertEqual(result.returncode, 0, result.stderr + result.stdout)
            self.assertEqual(result.stdout.splitlines()[0], "HEALTHY")
            self.assertIn("Expected transients", result.stdout)
            self.assertIn("disposition=expected-transient", result.stdout)
            self.assertIn("retry-pending:abc", result.stdout)
            self.assertIn("outcome=deadline-defer", result.stdout)
            self.assertIn("error_class=marker-lag", result.stdout)
            self.assertNotIn("ANOMALIES NEEDING ATTENTION", result.stdout)
        finally:
            h.close()

    def test_first_line_counts_terminal_dlq_and_stalled_non_terminal_entities(self) -> None:
        h = BoardHarness(
            {
                "entities": [
                    {
                        "entity": "github-devloop/issue/owner/repo/620",
                        "terminal": False,
                        "events": [{"queue": "devloop_ready", "ts": "2026-06-14T09:00:00Z"}],
                    },
                    {
                        "entity": "github-devloop/issue/owner/repo/621",
                        "terminal": True,
                        "events": [{"queue": "devloop_merged", "ts": "2026-06-14T08:00:00Z"}],
                    },
                ],
                "queues": [
                    {"queue": "devloop_ready", "ready": 0, "leased": 0, "retry": 0, "dlq": 1},
                    {"queue": "devloop_fixing", "ready": 0, "leased": 0, "retry": 2, "dlq": 0},
                ],
                "failure_facts": [
                    {
                        "schema": "fkst.failure_fact.v1",
                        "origin_queue": "devloop_fixing",
                        "origin_dept": "github-devloop.fix",
                        "error_class": "framework_child_nonzero",
                        "fingerprint": "framework_child_nonzero:ghi",
                        "attempt": 2,
                        "terminal": True,
                    }
                ],
            }
        )
        try:
            result = h.run_board("--refresh", "--stall", "1800")
            self.assertEqual(result.returncode, 0, result.stderr + result.stdout)
            self.assertEqual(result.stdout.splitlines()[0], "3 ANOMALIES NEEDING ATTENTION")
            self.assertIn("Anomalies needing attention", result.stdout)
            self.assertIn("type=terminal-failure", result.stdout)
            self.assertIn("type=stalled-entity", result.stdout)
            self.assertIn("type=queue-dlq", result.stdout)
            self.assertIn("framework_child_nonzero:ghi", result.stdout)
            self.assertNotIn("- github-devloop/issue/owner/repo/621 latest=devloop_merged dwell=2h0m", result.stdout)
        finally:
            h.close()

    def test_recurring_cron_dead_letters_surface_in_board_and_health(self) -> None:
        h = BoardHarness(
            {
                "failure_facts": [
                    {
                        "schema": "fkst.failure_fact.v1",
                        "origin_queue": "github-devloop.devloop_branch_tick",
                        "origin_dept": "github-devloop.sync_scan",
                        "source_ref": {"kind": "cron", "ref": ""},
                        "error_class": "framework_child_nonzero",
                        "fingerprint": "sync-scan:permission-denied",
                        "attempt": 5,
                        "terminal": True,
                    },
                    {
                        "schema": "fkst.failure_fact.v1",
                        "origin_queue": "github-devloop.devloop_branch_tick",
                        "origin_dept": "github-devloop.sync_scan",
                        "source_ref": {"kind": "cron", "ref": ""},
                        "error_class": "framework_child_nonzero",
                        "fingerprint": "sync-scan:permission-denied",
                        "attempt": 6,
                        "terminal": True,
                    },
                ],
                "queues": [{"queue": "github-devloop.devloop_branch_tick", "ready": 0, "leased": 0, "retry": 0, "dlq": 2}],
            }
        )
        try:
            board = h.run_board("--refresh", "--stall", "1800")
            self.assertEqual(board.returncode, 0, board.stderr + board.stdout)
            self.assertEqual(board.stdout.splitlines()[0], "1 ANOMALIES NEEDING ATTENTION")
            self.assertIn("type=infra-stall", board.stdout)
            self.assertIn("queue=github-devloop.devloop_branch_tick", board.stdout)
            self.assertIn("infra-stall:github-devloop.sync_scan", board.stdout)

            health = h.run_health("--stall", "1800")
            self.assertEqual(health.returncode, 0, health.stderr + health.stdout)
            self.assertEqual(health.stdout.strip(), "1 ANOMALIES NEEDING ATTENTION")
        finally:
            h.close()

    def test_health_subcommand_prints_compact_verdict_from_same_observe_cache(self) -> None:
        h = BoardHarness(
            {
                "entities": [
                    {
                        "entity": "github-devloop/issue/owner/repo/623",
                        "events": [{"queue": "devloop_ready", "ts": "2026-06-14T09:59:30Z"}],
                    }
                ],
                "queues": [{"queue": "devloop_ready", "ready": 0, "leased": 0, "retry": 1, "dlq": 0}],
            }
        )
        try:
            result = h.run_health("--refresh", "--stall", "1800")
            self.assertEqual(result.returncode, 0, result.stderr + result.stdout)
            self.assertEqual(result.stdout.strip(), "HEALTHY")
            self.assertIn("observe --project-root", h.calls())
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
