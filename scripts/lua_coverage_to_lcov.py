#!/usr/bin/env python3
"""Render canonical fkst Lua coverage JSON as LCOV.

This converter consumes the canonical fkst.lua.coverage.v1 artifact as the
single source of truth and does not recompute coverage from source. The
repository is public, so Codecov tokenless upload may work; CODECOV_TOKEN is an
optional operator-provided secret. Pull request comments and status checks
require the Codecov GitHub App to be installed by an operator. Those are
operator-side dependencies and are not assumed here.
"""

from __future__ import annotations

import json
import os
import sys
from pathlib import Path
from typing import Any


SCHEMA = "fkst.lua.coverage.v1"


def _require_files(data: dict[str, Any]) -> list[Any]:
    if data.get("schema") != SCHEMA:
        raise ValueError("schema must be fkst.lua.coverage.v1")
    files = data.get("files")
    if not isinstance(files, list):
        raise ValueError("files must be a list")
    return files


def _require_file_entry(entry: Any) -> tuple[str, list[Any]]:
    if not isinstance(entry, dict):
        raise ValueError("file entry must be an object")
    file = entry.get("file")
    if not isinstance(file, str):
        raise ValueError("file must be a string")
    coverable_lines = entry.get("coverable_lines")
    if not isinstance(coverable_lines, list):
        raise ValueError("coverable_lines must be a list")
    return file, coverable_lines


def _require_line_entry(entry: Any) -> tuple[int, bool]:
    if not isinstance(entry, dict):
        raise ValueError("line entry must be an object")
    line = entry.get("line")
    if isinstance(line, bool) or not isinstance(line, int) or line < 1:
        raise ValueError("line must be an integer >= 1")
    covered = entry.get("covered")
    if not isinstance(covered, bool):
        raise ValueError("covered must be a boolean")
    return line, covered


def render_lcov(data: dict[str, Any]) -> str:
    """Render fkst.lua.coverage.v1 data as deterministic LCOV."""
    files = _require_files(data)
    output: list[str] = []

    for file_entry in sorted(files, key=lambda item: _require_file_entry(item)[0]):
        file, coverable_lines = _require_file_entry(file_entry)
        rendered_lines = [_require_line_entry(line_entry) for line_entry in coverable_lines]
        rendered_lines.sort(key=lambda item: item[0])

        output.append(f"SF:{file}\n")
        covered_count = 0
        for line, covered in rendered_lines:
            if covered:
                covered_count += 1
            output.append(f"DA:{line},{1 if covered else 0}\n")
        output.append(f"LF:{len(rendered_lines)}\n")
        output.append(f"LH:{covered_count}\n")
        output.append("end_of_record\n")

    return "".join(output)


def main(argv: list[str]) -> int:
    if len(argv) > 2:
        raise SystemExit("usage: lua_coverage_to_lcov.py [input-json] [output-lcov]")
    input_arg = argv[0] if argv else os.environ.get("FKST_LUA_COVERAGE_OUTPUT")
    if not input_arg:
        raise SystemExit("error: input path missing; pass argv[1] or set FKST_LUA_COVERAGE_OUTPUT")

    data = json.loads(Path(input_arg).read_text(encoding="utf-8"))
    rendered = render_lcov(data)

    if len(argv) == 2:
        Path(argv[1]).write_text(rendered, encoding="utf-8")
    else:
        sys.stdout.write(rendered)
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
