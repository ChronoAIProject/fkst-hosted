#!/usr/bin/env python3
"""Peer cross-package require guard."""

from __future__ import annotations

import re
from pathlib import Path
from typing import Callable


REQUIRE_RE = re.compile(
    r"""\brequire\s*(?:\(\s*)?(?:"([A-Za-z0-9_.\-]+)"|'([A-Za-z0-9_.\-]+)'|\[(=*)\[([A-Za-z0-9_.\-]+)\]\3\])"""
)


def require_names(
    source: str,
    package_names: set[str],
    current_pkg: str,
    strip_lua_comments_and_strings: Callable[[str], str],
    is_unmasked_range: Callable[[str, str, int, int], bool],
) -> list[str]:
    """Top-level require names in `source` that name a sibling package."""
    hits: set[str] = set()
    stripped = strip_lua_comments_and_strings(source)
    for match in REQUIRE_RE.finditer(source):
        group = 1 if match.group(1) is not None else 2 if match.group(2) is not None else 4
        string_start = match.start(group) - 1
        string_end = match.end(group) + 1
        if group == 4:
            string_end += 1 + len(match.group(3))
        if not (is_unmasked_range(source, stripped, match.start(), string_start) and is_unmasked_range(source, stripped, string_end, match.end())):
            continue
        name = next(group for group in (match.group(1), match.group(2), match.group(4)) if group is not None)
        top = name.split(".")[0]
        if top in package_names and top != current_pkg:
            hits.add(top)
    return sorted(hits)


def messages(
    root: Path,
    package_dirs: Callable[[Path], list[Path]],
    read_text: Callable[[Path], str],
    rel: Callable[[Path, Path], str],
    strip_lua_comments_and_strings: Callable[[str], str],
    is_unmasked_range: Callable[[str, str, int, int], bool],
) -> list[str]:
    pkgs = package_dirs(root)
    names = {pkg.name for pkg in pkgs}
    violations: list[str] = []
    for pkg in pkgs:
        for path in sorted(pkg.rglob("*.lua")):
            if not path.is_file():
                continue
            parts = path.relative_to(pkg).parts
            if parts and parts[0] in {"std", "libraries"}:
                continue
            for name in require_names(read_text(path), names, pkg.name, strip_lua_comments_and_strings, is_unmasked_range):
                violations.append(
                    f"{rel(root, path)} peer cross-package require of {name!r}; share via workspace libraries (peer cross-package require is forbidden)"
                )
    return violations
