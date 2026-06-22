"""Repository guard against permission-based control."""

from __future__ import annotations

import re
from pathlib import Path
from typing import Callable


SOURCE_SUFFIXES = {".lua", ".sh", ".py"}
SCAN_DIRS = ("packages", "libraries", "scripts")
RESTRICTIVE_MODE_RE = re.compile(r"(?<![0-9A-Za-z_])(?:0o(?:555|444|500|400)|0(?:555|444|500|400))(?![0-9A-Za-z_])")
CHMOD_COMMAND_RE = re.compile(r"(?<![\w.-])chmod\s+(?!\+)")


def _is_test_path(path: Path) -> bool:
    return "tests" in path.parts or "_test." in path.name


def _allowed_chmod_line(line: str) -> bool:
    return CHMOD_COMMAND_RE.search(line) is None


def _source_paths(root: Path) -> list[Path]:
    paths: list[Path] = []
    for name in SCAN_DIRS:
        scan_root = root / name
        if not scan_root.exists():
            continue
        paths.extend(
            path
            for path in sorted(scan_root.rglob("*"))
            if path.is_file() and path.suffix in SOURCE_SUFFIXES
        )
    return paths


def check_no_permission_control(
    root: Path,
    violations: list[str],
    *,
    read_text: Callable[[Path], str] | None = None,
    rel: Callable[[Path, Path], str] | None = None,
) -> None:
    reader = read_text or (lambda path: path.read_text(encoding="utf-8"))
    rel_path = rel or (lambda base, path: path.relative_to(base).as_posix())
    for path in _source_paths(root):
        relative = path.relative_to(root)
        if _is_test_path(relative):
            continue
        for line_number, line in enumerate(reader(path).splitlines(), start=1):
            if not _allowed_chmod_line(line):
                violations.append(f"G-PERM: {rel_path(root, path)}:{line_number} permission command may not be used for permission-based control")
            if RESTRICTIVE_MODE_RE.search(line) is not None:
                violations.append(f"G-PERM: {rel_path(root, path)}:{line_number} restrictive mode literal may not be used for permission-based control")
