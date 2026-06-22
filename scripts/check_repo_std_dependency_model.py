#!/usr/bin/env python3
"""Std dependency-model guard for fkst packages."""

from __future__ import annotations

import json
import re
from pathlib import Path
from typing import Callable, Iterable

import ratchet_base


REQUIRE_LITERAL_RE = re.compile(
    r"""\brequire\s*(?:\(\s*)?(?P<quote>["'])(?P<module>[A-Za-z0-9_.\-]+)(?P=quote)"""
)
DEVLOOP_STD_IMPORTS_INVENTORY = "migration/devloop-std-imports.inventory"


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


def contract_module_exists(contract_root: Path, module: str) -> bool:
    if not module.startswith("contract."):
        return True
    module_path = contract_root.joinpath(*module.split(".")[1:])
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


def library_lua_files(root: Path, library_name: str) -> Iterable[Path]:
    library_root = root / "libraries" / library_name
    if not library_root.exists():
        return []
    return sorted(path for path in library_root.rglob("*.lua") if path.is_file())


def load_devloop_std_import_inventory_text(text: str, source: str) -> tuple[set[tuple[str, str]], list[str]]:
    entries: set[tuple[str, str]] = set()
    messages: list[str] = []
    for number, raw in enumerate(text.splitlines(), start=1):
        stripped = raw.strip()
        if stripped == "" or stripped.startswith("#"):
            continue
        try:
            doc = json.loads(stripped)
        except json.JSONDecodeError as exc:
            messages.append(f"{source}:{number}: invalid JSON: {exc.msg}")
            continue
        if not isinstance(doc, dict):
            messages.append(f"{source}:{number}: expected JSON object")
            continue
        path = doc.get("path")
        module = doc.get("module")
        if not isinstance(path, str) or not path.startswith("libraries/devloop/") or not path.endswith(".lua"):
            messages.append(f"{source}:{number}: path must be a libraries/devloop/*.lua path")
            continue
        if not isinstance(module, str) or not module.startswith("std."):
            messages.append(f"{source}:{number}: module must be a std.* module")
            continue
        entries.add((path, module))
    return entries, messages


def load_devloop_std_import_inventory(path: Path) -> tuple[set[tuple[str, str]], list[str]]:
    if not path.exists():
        return set(), [f"{DEVLOOP_STD_IMPORTS_INVENTORY} is required"]
    return load_devloop_std_import_inventory_text(path.read_text(encoding="utf-8"), DEVLOOP_STD_IMPORTS_INVENTORY)


def devloop_std_imports_at_base(root: Path) -> tuple[str, set[tuple[str, str]] | None, list[str]]:
    status, text = ratchet_base.file_at_base(root, DEVLOOP_STD_IMPORTS_INVENTORY)
    if status == "absent":
        return status, None, []
    if status == "unresolved" or text is None:
        return "unresolved", None, []
    entries, messages = load_devloop_std_import_inventory_text(text, f"base:{DEVLOOP_STD_IMPORTS_INVENTORY}")
    return "present", entries, messages


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
    contract_root = root / "libraries" / "contract"
    package_usage: dict[str, set[str]] = {package.name: set() for package in packages}
    std_edges: dict[str, set[str]] = {}
    devloop_std_imports: set[tuple[str, str]] = set()
    devloop_files = list(library_lua_files(root, "devloop"))

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
                if module.startswith("contract."):
                    if not contract_module_exists(contract_root, module):
                        add(violations, "G-STD-DEP", f"{rel(root, path)}:{line} requires unresolved module {module!r}")
                    continue
                if module.startswith("devloop."):
                    add(violations, "G-STD-DEP", f"{rel(root, path)}:{line} std/forge must not require {module!r}")
                    continue
                if module != "std":
                    add(
                        violations,
                        "G-STD-DEP",
                        f'std module {rel(root, path)}:{line} requires non-std module "{module}" '
                        "(std must receive resolved values from package-owned wiring)",
                    )

    if contract_root.exists():
        for path in sorted(contract_root.rglob("*.lua")):
            if not path.is_file():
                continue
            for module, line in require_literals(read_text(path), strip_lua_comments_and_strings, is_unmasked_range):
                if module.startswith("contract."):
                    if not contract_module_exists(contract_root, module):
                        add(violations, "G-STD-DEP", f"{rel(root, path)}:{line} requires unresolved module {module!r}")
                    continue
                if module.startswith("std.") or module.startswith("devloop."):
                    add(violations, "G-STD-DEP", f"{rel(root, path)}:{line} contract must not require {module!r}")
                    continue
                if module != "contract":
                    add(
                        violations,
                        "G-STD-DEP",
                        f'contract module {rel(root, path)}:{line} requires non-contract module "{module}" '
                        "(contract must be a pure base library)",
                    )

    for path in devloop_files:
        rel_path = rel(root, path)
        for module, line in require_literals(read_text(path), strip_lua_comments_and_strings, is_unmasked_range):
            if module.startswith("contract."):
                if not contract_module_exists(contract_root, module):
                    add(violations, "G-STD-DEP", f"{rel_path}:{line} requires unresolved module {module!r}")
                continue
            if module.startswith("std."):
                if not module_exists(std_root, module):
                    add(violations, "G-STD-DEP", f"{rel_path}:{line} requires unresolved module {module!r}")
                devloop_std_imports.add((rel_path, module))
                continue

    for package in packages:
        for path in package_lua_files(package):
            for module, line in require_literals(read_text(path), strip_lua_comments_and_strings, is_unmasked_range):
                if module.startswith("contract."):
                    package_usage[package.name].add(module)
                    if not contract_module_exists(contract_root, module):
                        add(violations, "G-STD-DEP", f"{rel(root, path)}:{line} requires unresolved module {module!r}")
                    continue
                if not module.startswith("std."):
                    continue
                package_usage[package.name].add(module)
                if not module_exists(std_root, module):
                    add(violations, "G-STD-DEP", f"{rel(root, path)}:{line} requires unresolved module {module!r}")

    inventory_path = root / DEVLOOP_STD_IMPORTS_INVENTORY
    if devloop_files or inventory_path.exists():
        current_inventory, inventory_errors = load_devloop_std_import_inventory(inventory_path)
        for message in inventory_errors:
            add(violations, "G-STD-DEP", message)
        for item in sorted(devloop_std_imports - current_inventory):
            path, module = item
            add(violations, "G-STD-DEP", f"{path} imports {module} but is not listed in {DEVLOOP_STD_IMPORTS_INVENTORY}")
        for item in sorted(current_inventory - devloop_std_imports):
            path, module = item
            add(violations, "G-STD-DEP", f"{DEVLOOP_STD_IMPORTS_INVENTORY} lists stale import {path} {module}")
        base_status, base_inventory, base_errors = devloop_std_imports_at_base(root)
        for message in base_errors:
            add(violations, "G-STD-DEP", message)
        if base_status == "unresolved":
            add(violations, "G-STD-DEP", f"cannot resolve dev base {DEVLOOP_STD_IMPORTS_INVENTORY} to enforce shrink-only ratchet")
        elif base_inventory is not None:
            for item in sorted(current_inventory - base_inventory):
                path, module = item
                add(violations, "G-STD-DEP", f"{DEVLOOP_STD_IMPORTS_INVENTORY} grows relative to dev: {path} {module}")

    for package_name, modules in sorted(package_usage.items()):
        if modules:
            add(warnings, "G-STD-DEP", f"package {package_name} uses {', '.join(sorted(modules))}")
    for source_module, modules in sorted(std_edges.items()):
        if modules:
            add(warnings, "G-STD-DEP", f"std internal edge {source_module} -> {', '.join(sorted(modules))}")
