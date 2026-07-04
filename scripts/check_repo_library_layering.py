#!/usr/bin/env python3
"""Guard that shared libraries never require behavior-layer package code."""

from __future__ import annotations

from pathlib import Path
from typing import Callable

import check_repo_cross_package
import check_repo_std_dependency_model
import ratchet_base


ALLOWLIST = "migration/library-layering.allowlist"
PACKAGE_ONLY_TOP_LEVELS = {"core", "departments", "tests", "raisers"}


def workspace_library_names(root: Path) -> set[str]:
    libraries = root / "libraries"
    if not libraries.exists():
        return set()
    return {
        path.name
        for path in sorted(libraries.iterdir())
        if path.is_dir() and (path / "fkst.toml").exists()
    }


def package_names(root: Path, package_dirs: Callable[[Path], list[Path]]) -> set[str]:
    return {path.name for path in package_dirs(root)}


def library_lua_files(root: Path) -> list[Path]:
    libraries = root / "libraries"
    if not libraries.exists():
        return []
    return sorted(path for path in libraries.rglob("*.lua") if path.is_file())


def require_literals(
    source: str,
    strip_lua_comments_and_strings: Callable[[str], str],
    is_unmasked_range: Callable[[str, str, int, int], bool],
) -> list[tuple[str, int]]:
    return check_repo_std_dependency_model.require_literals(
        source,
        strip_lua_comments_and_strings,
        is_unmasked_range,
    )


def forbidden_reason(module: str, libraries: set[str], packages: set[str]) -> str | None:
    top = module.split(".")[0]
    if top in libraries:
        return None
    if top in PACKAGE_ONLY_TOP_LEVELS:
        return "package-only module"
    if top in packages:
        return "package namespace"
    return None


def current_sites(
    root: Path,
    package_dirs: Callable[[Path], list[Path]],
    read_text: Callable[[Path], str],
    rel: Callable[[Path, Path], str],
    strip_lua_comments_and_strings: Callable[[str], str],
    is_unmasked_range: Callable[[str, str, int, int], bool],
) -> set[str]:
    libraries = workspace_library_names(root)
    packages = package_names(root, package_dirs)
    sites: set[str] = set()
    for path in library_lua_files(root):
        for module, line in require_literals(read_text(path), strip_lua_comments_and_strings, is_unmasked_range):
            reason = forbidden_reason(module, libraries, packages)
            if reason is not None:
                sites.add(f"{rel(root, path)}:{line}:{module}:{reason}")
    return sites


def parse_allowlist_lines(lines: list[str]) -> set[str]:
    entries: set[str] = set()
    for raw in lines:
        line = raw.split("#", 1)[0].strip()
        if not line:
            continue
        parts = line.split(":")
        if len(parts) != 4:
            raise ValueError(f"invalid {ALLOWLIST} line: {raw}")
        path, line_no, module, reason = parts
        if not path.startswith("libraries/") or not path.endswith(".lua"):
            raise ValueError(f"invalid {ALLOWLIST} line: {raw}")
        if not line_no.isdigit() or int(line_no) < 1:
            raise ValueError(f"invalid {ALLOWLIST} line: {raw}")
        if not module or reason not in {"package-only module", "package namespace"}:
            raise ValueError(f"invalid {ALLOWLIST} line: {raw}")
        entries.add(line)
    return entries


def load_allowlist(path: Path) -> set[str]:
    if not path.exists():
        return set()
    return parse_allowlist_lines(path.read_text(encoding="utf-8").splitlines())


def allowlist_at_dev_base(root: Path) -> tuple[str, set[str] | None]:
    try:
        status, shown = ratchet_base.file_at_base(root, ALLOWLIST)
        if status != "present":
            return status, None
        assert shown is not None
        return "present", parse_allowlist_lines(shown.splitlines())
    except Exception:
        return "unresolved", None


def site_message(site: str) -> str:
    path, line, module, reason = site.split(":", 3)
    return (
        f"{path}:{line} library requires {reason} {module!r}; "
        "libraries are the lower shared layer and must depend only on workspace libraries or external Lua modules"
    )


def ratchet_messages(
    current: set[str],
    allowlist: set[str],
    base_allowlist: set[str] | None = None,
) -> list[str]:
    messages = [
        f"{site_message(site)}; not in {ALLOWLIST}"
        for site in sorted(current - allowlist)
    ]
    messages.extend(
        f"{site} is stale in {ALLOWLIST}; prune the stale entry"
        for site in sorted(allowlist - current)
    )
    if base_allowlist is not None:
        messages.extend(
            f"{site} grows {ALLOWLIST} relative to dev; remove the libraries-to-packages require instead"
            for site in sorted(allowlist - base_allowlist)
        )
    return messages


def messages(
    root: Path,
    package_dirs: Callable[[Path], list[Path]],
    read_text: Callable[[Path], str],
    rel: Callable[[Path, Path], str],
    strip_lua_comments_and_strings: Callable[[str], str],
    is_unmasked_range: Callable[[str, str, int, int], bool],
    allowlist_dir: Path | None = None,
    enforce_base: bool = True,
) -> list[str]:
    current = current_sites(root, package_dirs, read_text, rel, strip_lua_comments_and_strings, is_unmasked_range)
    allowlist_path = root / ALLOWLIST if allowlist_dir is None else allowlist_dir / Path(ALLOWLIST).name
    allowlist = load_allowlist(allowlist_path)
    base_status, base_allowlist = allowlist_at_dev_base(root) if enforce_base else ("absent", None)
    result: list[str] = []
    if base_status == "unresolved":
        result.append("cannot resolve dev base allowlist to enforce shrink-only library-layering ratchet; ensure CI provides the dev ref")
    result.extend(ratchet_messages(current, allowlist, base_allowlist))
    return result
