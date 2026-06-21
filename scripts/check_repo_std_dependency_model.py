#!/usr/bin/env python3
"""Std dependency-model guard for fkst packages."""

from __future__ import annotations

import re
from pathlib import Path
from typing import Callable, Iterable


REQUIRE_LITERAL_RE = re.compile(
    r"""\brequire\s*(?:\(\s*)?(?P<quote>["'])(?P<module>[A-Za-z0-9_.\-]+)(?P=quote)"""
)


def require_literals(
    source: str,
    strip_lua_comments_and_strings: Callable[[str], str],
    is_unmasked_range: Callable[[str, str, int, int], bool],
) -> list[tuple[str, int]]:
    stripped = strip_lua_comments_and_strings(source)
    found: list[tuple[str, int]] = []
    for match in REQUIRE_LITERAL_RE.finditer(source):
        if not is_unmasked_range(source, stripped, match.start(), match.start("quote")):
            continue
        found.append((match.group("module"), source.count("\n", 0, match.start()) + 1))
    return found


def module_exists(std_root: Path, module: str) -> bool:
    if not module.startswith("std."):
        return True
    module_path = std_root.joinpath(*module.split(".")[1:])
    return module_path.with_suffix(".lua").is_file() or (module_path / "init.lua").is_file()


def std_module_name(std_root: Path, path: Path) -> str:
    relative = path.relative_to(std_root)
    if relative.name == "init.lua":
        parts = relative.parent.parts
    else:
        parts = relative.with_suffix("").parts
    return ".".join(("std", *parts)) if parts else "std"


def package_lua_files(package_root: Path) -> Iterable[Path]:
    for path in sorted(package_root.rglob("*.lua")):
        if not path.is_file():
            continue
        parts = path.relative_to(package_root).parts
        if parts and parts[0] == "std":
            continue
        yield path


def check_std_dependency_model(
    root: Path,
    violations: list[str],
    warnings: list[str],
    *,
    packages: Iterable[Path],
    read_text: Callable[[Path], str],
    rel: Callable[[Path, Path], str],
    add: Callable[[list[str], str, str], None],
    strip_lua_comments_and_strings: Callable[[str], str],
    is_unmasked_range: Callable[[str, str, int, int], bool],
) -> None:
    packages = list(packages)
    std_root = root / "std"
    package_usage: dict[str, set[str]] = {package.name: set() for package in packages}
    std_edges: dict[str, set[str]] = {}

    if std_root.exists():
        for path in sorted(std_root.rglob("*.lua")):
            if not path.is_file():
                continue
            source_module = std_module_name(std_root, path)
            for module, line in require_literals(read_text(path), strip_lua_comments_and_strings, is_unmasked_range):
                if module.startswith("std."):
                    if not module_exists(std_root, module):
                        add(violations, "G-STD-DEP", f"{rel(root, path)}:{line} requires unresolved module {module!r}")
                    std_edges.setdefault(source_module, set()).add(module)
                    continue
                if module != "std":
                    # std-depends-only-on-std is the ADR-0001 intent, but the
                    # codebase deliberately uses a template-method inversion where
                    # a shared std module (e.g. std/devloop_prompts.lua) requires a
                    # bare `prompts.<name>` that resolves to the CONSUMING package's
                    # own module. Surface it as a report-only finding (ratchet: warn
                    # now, promote to a violation if/when that pattern is removed),
                    # rather than failing CI on an intentional existing pattern.
                    add(
                        warnings,
                        "G-STD-DEP",
                        f'std module {rel(root, path)}:{line} requires non-std module "{module}" '
                        "(resolves to consuming-package code; std should ideally depend only on std)",
                    )

    for package in packages:
        for path in package_lua_files(package):
            for module, line in require_literals(read_text(path), strip_lua_comments_and_strings, is_unmasked_range):
                if not module.startswith("std."):
                    continue
                package_usage[package.name].add(module)
                if not module_exists(std_root, module):
                    add(violations, "G-STD-DEP", f"{rel(root, path)}:{line} requires unresolved module {module!r}")

    for package_name, modules in sorted(package_usage.items()):
        if modules:
            add(warnings, "G-STD-DEP", f"package {package_name} uses {', '.join(sorted(modules))}")
    for source_module, modules in sorted(std_edges.items()):
        if modules:
            add(warnings, "G-STD-DEP", f"std internal edge {source_module} -> {', '.join(sorted(modules))}")
