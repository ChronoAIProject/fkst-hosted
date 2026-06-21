#!/usr/bin/env python3
"""Tests for bare own-queue compare repository guard."""

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
namespaced_queue = check_repo.check_repo_namespaced_queue


class NamespacedQueueGuardTest(unittest.TestCase):
    def messages(self, source: str) -> list[str]:
        return namespaced_queue.bare_own_queue_compare_messages(
            "packages/example/departments/gate/main.lua",
            textwrap.dedent(source),
            check_repo.strip_lua_comments_and_strings,
            check_repo.is_unmasked_range,
        )

    def test_flags_bare_own_queue_equality_compare(self) -> None:
        messages = self.messages(
            """\
            local spec = {
              consumes = { "idle_tick" },
            }

            local function done(event)
              return event.queue == "idle_tick"
            end
            """
        )

        self.assertEqual(len(messages), 1)
        self.assertIn("compares event.queue to bare own queue 'idle_tick'", messages[0])

    def test_flags_bare_own_queue_inequality_compare(self) -> None:
        messages = self.messages(
            """\
            local spec = {
              consumes = { "idle_tick" },
            }

            local function done(event)
              if event.queue ~= "idle_tick" then
                error("example: unknown-queue: unsupported event")
              end
              return false
            end
            """
        )

        self.assertEqual(len(messages), 1)
        self.assertIn(":6 ", messages[0])

    def test_flags_reversed_bare_own_queue_compare(self) -> None:
        messages = self.messages(
            """\
            local spec = {
              consumes = { "idle_tick" },
            }

            local function done(event)
              return "idle_tick" == event.queue
            end
            """
        )

        self.assertEqual(len(messages), 1)
        self.assertIn("compares event.queue to bare own queue 'idle_tick'", messages[0])

    def test_flags_bracket_bare_own_queue_compare(self) -> None:
        messages = self.messages(
            """\
            local spec = {
              consumes = { "idle_tick" },
            }

            local function done(event)
              return event["queue"] == "idle_tick"
            end
            """
        )

        self.assertEqual(len(messages), 1)
        self.assertIn("compares event.queue to bare own queue 'idle_tick'", messages[0])

    def test_allows_fully_namespaced_compare(self) -> None:
        self.assertEqual(
            self.messages(
                """\
                local spec = {
                  consumes = { "idle-detector.system_idle" },
                }

                local function done(event)
                  return event.queue == "idle-detector.system_idle"
                end
                """
            ),
            [],
        )

    def test_allows_fully_namespaced_compare_with_bare_consumes_present(self) -> None:
        self.assertEqual(
            self.messages(
                """\
                local spec = {
                  consumes = { "system_idle" },
                }

                local function done(event)
                  return event.queue == "idle-detector.system_idle"
                end
                """
            ),
            [],
        )

    def test_allows_bare_normalization_helpers_and_other_queues(self) -> None:
        self.assertEqual(
            self.messages(
                """\
                local spec = {
                  consumes = { "idle_tick" },
                }

                local function normalize(queue)
                  if queue == "idle_tick" then
                    return "idle-detector.idle_tick"
                  end
                end

                local function done(event)
                  return event.queue == "other_tick"
                end
                """
            ),
            [],
        )

    def test_ignores_comments_and_strings(self) -> None:
        self.assertEqual(
            self.messages(
                """\
                local spec = {
                  consumes = { "idle_tick" },
                }

                -- if event.queue == "idle_tick" then return true end
                local text = 'event.queue ~= "idle_tick"'
                local function done(_event)
                  return false
                end
                """
            ),
            [],
        )


if __name__ == "__main__":
    unittest.main()
