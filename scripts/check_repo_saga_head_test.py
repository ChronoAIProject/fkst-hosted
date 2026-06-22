#!/usr/bin/env python3
"""Unit tests for G-SAGA-HEAD repository guard."""

from __future__ import annotations

import importlib.util
import sys
import textwrap
import unittest
from pathlib import Path


def load_module(name: str, path: Path):
    spec = importlib.util.spec_from_file_location(name, path)
    if spec is None or spec.loader is None:
        raise RuntimeError(f"could not load {path}")
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


scripts = Path(__file__).resolve().parent
check_repo = load_module("check_repo", scripts / "check_repo.py")
saga_head = check_repo.check_repo_saga_head


class SagaSpecHeadRatchetTest(unittest.TestCase):
    slice_1186_paths = {
        "packages/github-devloop-pr/departments/review_meta/main.lua",
        "packages/github-devloop-pr/departments/review_pr/main.lua",
        "packages/github-devloop-pr/departments/review_result/main.lua",
    }

    def violations(self, source: str) -> list[str]:
        return saga_head.violations(
            {"packages/example/departments/dept/main.lua": source},
            check_repo.strip_lua_comments_and_strings,
        )

    def test_spec_at_head_passes(self) -> None:
        source = textwrap.dedent(
            """\
            local core = require("core")
            local saga = require("workflow.saga")

            local spec = {
              consumes = { "q" },
              produces = { "done" },
              stall_window = "2m",
            }

            local function done(_event)
              return false
            end

            local function act(event)
              return event
            end

            return saga.department(spec, {
              done = done,
              act = act,
              wrap = core.wrap_pipeline_failure,
              name = "dept",
            })
            """
        )

        self.assertEqual(self.violations(source), [])

    def test_spec_at_bottom_fails(self) -> None:
        source = textwrap.dedent(
            """\
            local saga = require("workflow.saga")

            local function done(_event)
              return false
            end

            local function act(event)
              return event
            end

            local spec = {
              consumes = { "q" },
            }

            return saga.department(spec, {
              done = done,
              act = act,
            })
            """
        )

        violations = self.violations(source)
        self.assertEqual(len(violations), 1)
        self.assertIn("must be declared before the first local function", violations[0])

    def test_inline_spec_fails(self) -> None:
        source = textwrap.dedent(
            """\
            local saga = require("workflow.saga")

            local function done(_event)
              return false
            end

            local function act(event)
              return event
            end

            return saga.department({
              consumes = { "q" },
            }, {
              done = done,
              act = act,
            })
            """
        )

        violations = self.violations(source)
        self.assertEqual(len(violations), 1)
        self.assertIn("must pass a named spec first argument", violations[0])

    def test_issue_1186_slice_is_saga_shaped_and_allowlist_pruned(self) -> None:
        root = Path(__file__).resolve().parents[1]
        sources = {
            path: (root / path).read_text(encoding="utf-8")
            for path in sorted(self.slice_1186_paths)
        }
        allowlist = {
            line.strip()
            for line in (root / "migration" / "saga-handler.allowlist").read_text(encoding="utf-8").splitlines()
            if line.strip() and not line.lstrip().startswith("#")
        }

        violations = check_repo.saga_handler_ratchet_violations(sources, allowlist)

        self.assertEqual(
            [message for message in violations if any(path in message for path in self.slice_1186_paths)],
            [],
        )
        self.assertTrue(self.slice_1186_paths.isdisjoint(allowlist))


if __name__ == "__main__":
    unittest.main()
