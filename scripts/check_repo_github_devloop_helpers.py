#!/usr/bin/env python3
"""github-devloop-specific helper ownership guards."""

from __future__ import annotations

import re
from pathlib import Path
from typing import Callable

NAME_ONLY_PATHS_LOCAL_HELPER_RE = re.compile(r"\blocal\s+function\s+parse_name_only_paths\s*\(")
RECURRENCE_API_WAIVER_TOKEN = "Recurrence/API waiver"


def messages(sources: dict[str, str], strip_lua_comments_and_strings) -> list[str]:
    reports: list[str] = []
    core_path = "packages/github-devloop/core.lua"
    for path, source in sorted(sources.items()):
        if not path.startswith("packages/github-devloop/"):
            continue
        if "/tests/" in path or path == core_path:
            continue
        stripped = strip_lua_comments_and_strings(source)
        for match in NAME_ONLY_PATHS_LOCAL_HELPER_RE.finditer(stripped):
            line = source.count("\n", 0, match.start()) + 1
            reports.append(
                f"{path}:{line} local parse_name_only_paths helper is forbidden; use core.parse_name_only_paths or add a reviewed Recurrence/API waiver"
            )
    return reports


def repository_messages(
    root: Path,
    packages: Path,
    read_text: Callable[[Path], str],
    rel: Callable[[Path, Path], str],
    strip_lua_comments_and_strings,
) -> list[str]:
    package_root = packages / "github-devloop"
    if not package_root.exists():
        return []
    sources = {
        rel(root, path): read_text(path)
        for path in sorted(package_root.rglob("*.lua"))
        if path.is_file()
    }
    return messages(sources, strip_lua_comments_and_strings)
