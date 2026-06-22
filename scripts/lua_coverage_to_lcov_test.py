#!/usr/bin/env python3
"""Unit tests for rendering canonical Lua coverage as LCOV."""

from __future__ import annotations

import contextlib
import importlib.util
import io
import json
import os
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


scripts = Path(__file__).resolve().parent
lcov = load_module("lua_coverage_to_lcov", scripts / "lua_coverage_to_lcov.py")


class LuaCoverageToLcovTest(unittest.TestCase):
    def artifact(self, files: list[dict[str, object]]) -> dict[str, object]:
        return {"schema": "fkst.lua.coverage.v1", "files": files}

    def test_maps_covered_and_uncovered_lines_to_da_counts(self) -> None:
        rendered = lcov.render_lcov(
            self.artifact(
                [
                    {
                        "file": "packages/example/core.lua",
                        "coverable_lines": [
                            {
                                "line": 12,
                                "normalized_line_hash": "a1",
                                "text": "x()",
                                "covered": True,
                            },
                            {
                                "line": 18,
                                "normalized_line_hash": "b2",
                                "text": "y()",
                                "covered": False,
                            },
                        ],
                    }
                ]
            )
        )

        self.assertEqual(
            rendered,
            "\n".join(
                [
                    "SF:packages/example/core.lua",
                    "DA:12,1",
                    "DA:18,0",
                    "LF:2",
                    "LH:1",
                    "end_of_record",
                    "",
                ]
            ),
        )

    def test_file_with_zero_covered_lines_has_zero_lh(self) -> None:
        rendered = lcov.render_lcov(
            self.artifact(
                [
                    {
                        "file": "packages/example/core.lua",
                        "coverable_lines": [
                            {
                                "line": 2,
                                "normalized_line_hash": "a1",
                                "text": "x()",
                                "covered": False,
                            }
                        ],
                    }
                ]
            )
        )

        self.assertIn("LF:1\nLH:0\n", rendered)

    def test_empty_files_render_empty_output(self) -> None:
        self.assertEqual(lcov.render_lcov(self.artifact([])), "")

    def test_repo_relative_paths_are_preserved(self) -> None:
        rendered = lcov.render_lcov(
            self.artifact(
                [
                    {
                        "file": "libraries/contract/strings.lua",
                        "coverable_lines": [
                            {
                                "line": 3,
                                "normalized_line_hash": "a1",
                                "text": "return value",
                                "covered": True,
                            }
                        ],
                    },
                    {
                        "file": "packages/example/core.lua",
                        "coverable_lines": [
                            {
                                "line": 7,
                                "normalized_line_hash": "b2",
                                "text": "return value",
                                "covered": True,
                            }
                        ],
                    },
                ]
            )
        )

        self.assertIn("SF:packages/example/core.lua\n", rendered)
        self.assertIn("SF:libraries/contract/strings.lua\n", rendered)

    def test_output_is_byte_for_byte_deterministic_and_sorted(self) -> None:
        data = self.artifact(
            [
                {
                    "file": "libraries/contract/strings.lua",
                    "coverable_lines": [
                        {
                            "line": 9,
                            "normalized_line_hash": "c3",
                            "text": "z()",
                            "covered": True,
                        },
                        {
                            "line": 4,
                            "normalized_line_hash": "d4",
                            "text": "a()",
                            "covered": False,
                        },
                    ],
                },
                {
                    "file": "packages/example/core.lua",
                    "coverable_lines": [
                        {
                            "line": 2,
                            "normalized_line_hash": "a1",
                            "text": "x()",
                            "covered": True,
                        }
                    ],
                },
            ]
        )

        first = lcov.render_lcov(data)
        second = lcov.render_lcov(data)

        self.assertEqual(first, second)
        self.assertLess(first.index("SF:libraries/contract/strings.lua"), first.index("SF:packages/example/core.lua"))
        self.assertLess(first.index("DA:4,0"), first.index("DA:9,1"))

    def test_invalid_schema_raises_narrow_value_error(self) -> None:
        with self.assertRaisesRegex(ValueError, "schema"):
            lcov.render_lcov({"schema": "other", "files": []})

    def test_missing_covered_raises_narrow_value_error(self) -> None:
        with self.assertRaisesRegex(ValueError, "covered"):
            lcov.render_lcov(
                self.artifact(
                    [
                        {
                            "file": "packages/example/core.lua",
                            "coverable_lines": [
                                {
                                    "line": 2,
                                    "normalized_line_hash": "a1",
                                    "text": "x()",
                                }
                            ],
                        }
                    ]
                )
            )

    def test_non_int_line_raises_narrow_value_error(self) -> None:
        with self.assertRaisesRegex(ValueError, "line"):
            lcov.render_lcov(
                self.artifact(
                    [
                        {
                            "file": "packages/example/core.lua",
                            "coverable_lines": [
                                {
                                    "line": "2",
                                    "normalized_line_hash": "a1",
                                    "text": "x()",
                                    "covered": True,
                                }
                            ],
                        }
                    ]
                )
            )


class MainCliTest(unittest.TestCase):
    def setUp(self) -> None:
        self.artifact = {
            "schema": "fkst.lua.coverage.v1",
            "files": [
                {
                    "file": "libraries/contract/strings.lua",
                    "coverable_lines": [
                        {"line": 3, "normalized_line_hash": "a1", "text": "x()", "covered": True},
                        {"line": 5, "normalized_line_hash": "b2", "text": "y()", "covered": False},
                    ],
                }
            ],
        }
        self.expected = lcov.render_lcov(self.artifact)

    def write_input(self, directory: str) -> str:
        path = Path(directory) / "coverage.json"
        path.write_text(json.dumps(self.artifact), encoding="utf-8")
        return str(path)

    def test_main_writes_lcov_to_output_file(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            input_path = self.write_input(directory)
            output_path = str(Path(directory) / "lcov.info")
            rc = lcov.main([input_path, output_path])
            self.assertEqual(rc, 0)
            self.assertEqual(Path(output_path).read_text(encoding="utf-8"), self.expected)

    def test_main_writes_lcov_to_stdout_when_no_output_path(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            input_path = self.write_input(directory)
            buffer = io.StringIO()
            with contextlib.redirect_stdout(buffer):
                rc = lcov.main([input_path])
            self.assertEqual(rc, 0)
            self.assertEqual(buffer.getvalue(), self.expected)

    def test_main_reads_input_from_env_when_no_argv(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            input_path = self.write_input(directory)
            buffer = io.StringIO()
            with mock.patch.dict(os.environ, {"FKST_LUA_COVERAGE_OUTPUT": input_path}, clear=False):
                with contextlib.redirect_stdout(buffer):
                    rc = lcov.main([])
            self.assertEqual(rc, 0)
            self.assertEqual(buffer.getvalue(), self.expected)

    def test_main_without_input_exits_with_narrow_error(self) -> None:
        with mock.patch.dict(os.environ, {}, clear=True):
            with self.assertRaises(SystemExit) as caught:
                lcov.main([])
        self.assertIn("input", str(caught.exception))

    def test_main_with_too_many_args_exits(self) -> None:
        with self.assertRaises(SystemExit) as caught:
            lcov.main(["a", "b", "c"])
        self.assertIn("usage", str(caught.exception))


if __name__ == "__main__":
    unittest.main()
