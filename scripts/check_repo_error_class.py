"""Shrink-only ratchet for unclassified production error() strings."""

from __future__ import annotations

from pathlib import Path

import ratchet_base


ALLOWLIST = "migration/error-class.allowlist"


def parse_allowlist_lines(lines: list[str]) -> set[str]:
    entries: set[str] = set()
    for raw in lines:
        line = raw.split("#", 1)[0].strip()
        if not line:
            continue
        if not line.startswith("packages/") or ":line=" in line or ":" not in line:
            raise ValueError(f"invalid {ALLOWLIST} line: {raw}")
        path_part, line_part = line.rsplit(":", 1)
        if not path_part.endswith(".lua") or not line_part.isdigit() or int(line_part) < 1:
            raise ValueError(f"invalid {ALLOWLIST} line: {raw}")
        entries.add(line)
    return entries


def load_allowlist(path: Path) -> set[str]:
    if not path.exists():
        return set()
    return parse_allowlist_lines(path.read_text(encoding="utf-8").splitlines())


def current_sites(root, package_lua_files, read_text, rel, unclassified_error_call_lines) -> set[str]:
    sites: set[str] = set()
    for packages, path in package_lua_files(root):
        if not path.is_file() or "tests" in path.relative_to(packages).parts:
            continue
        for line in unclassified_error_call_lines(read_text(path)):
            sites.add(f"{rel(root, path)}:{line}")
    return sites


def ratchet_messages(
    current: set[str],
    allowlist: set[str],
    base_allowlist: set[str] | None = None,
) -> list[str]:
    messages = [
        f"{site} production error(...) string lacks a greppable class prefix and is not in {ALLOWLIST}"
        for site in sorted(current - allowlist)
    ]
    if base_allowlist is not None:
        messages.extend(
            f"{site} grows {ALLOWLIST} relative to dev; classify the error string instead"
            for site in sorted(allowlist - base_allowlist)
        )
    return messages


def allowlist_at_dev_base(root: Path) -> tuple[str, set[str] | None]:
    try:
        status, shown = ratchet_base.file_at_base(root, ALLOWLIST)
        if status != "present":
            return status, None
        assert shown is not None
        return "present", parse_allowlist_lines(shown.splitlines())
    except Exception:
        return "unresolved", None
