#!/usr/bin/env python3
"""Positive library dependency-model guards for fkst packages."""

from __future__ import annotations

import json
import re
from pathlib import Path
from typing import Callable, Iterable

import ratchet_base


REQUIRE_LITERAL_RE = re.compile(
    r"""\brequire\s*(?:\(\s*)?(?P<quote>["'])(?P<module>[A-Za-z0-9_.\-]+)(?P=quote)"""
)
DEVLOOP_FORGE_IMPORTS_INVENTORY = "migration/devloop-forge-imports.inventory"
LEGACY_DEVLOOP_STD_IMPORTS_INVENTORY = "migration/devloop-std-imports.inventory"
DEVLOOP_FORGE_VALIDATORS_FACADE = "libraries/devloop/forge_validators.lua"
DEVLOOP_FORGE_VALIDATOR_MODULES = {
    "forge.gitref",
}
FORGE_STRINGS_SPLIT_IMPORTS = {
    ("libraries/devloop/parsers/misc.lua", "forge.strings"),
}
DEVLOOP_FAMILY = {
    "fkst-substrate-ref-maintainer",
    "github-devloop",
    "github-devloop-decompose",
    "github-devloop-intake-default",
    "github-devloop-intake",
    "github-devloop-integration",
    "github-devloop-ops",
    "github-devloop-pr",
}


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


def library_lua_files(root: Path, library_name: str) -> list[Path]:
    library_root = root / "libraries" / library_name
    if not library_root.exists():
        return []
    return sorted(path for path in library_root.rglob("*.lua") if path.is_file())


def load_visibility_allow(path: Path) -> set[str]:
    if not path.exists():
        return set()
    text = path.read_text(encoding="utf-8")
    match = re.search(r"(?ms)^\[visibility\]\s*\n\s*allow\s*=\s*\[(?P<body>.*?)\]", text)
    if match is None:
        return set()
    return set(re.findall(r"[\"']([A-Za-z0-9_.-]+)[\"']", match.group("body")))


def canonical_forge_module(module: str) -> str:
    if module.startswith("std."):
        return "forge." + module[len("std."):]
    return module


def load_devloop_forge_import_inventory_text(text: str, source: str) -> tuple[set[tuple[str, str]], list[str]]:
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
        module = canonical_forge_module(str(doc.get("module") or ""))
        if not isinstance(path, str) or not path.startswith("libraries/devloop/") or not path.endswith(".lua"):
            messages.append(f"{source}:{number}: path must be a libraries/devloop/*.lua path")
            continue
        if not module.startswith("forge."):
            messages.append(f"{source}:{number}: module must be a forge.* module")
            continue
        entries.add((path, module))
    return entries, messages


def load_devloop_forge_import_inventory(path: Path) -> tuple[set[tuple[str, str]], list[str]]:
    if not path.exists():
        return set(), [f"{DEVLOOP_FORGE_IMPORTS_INVENTORY} is required"]
    return load_devloop_forge_import_inventory_text(path.read_text(encoding="utf-8"), DEVLOOP_FORGE_IMPORTS_INVENTORY)


def devloop_forge_imports_at_base(root: Path) -> tuple[str, set[tuple[str, str]] | None, list[str]]:
    status, text = ratchet_base.file_at_base(root, DEVLOOP_FORGE_IMPORTS_INVENTORY)
    source = DEVLOOP_FORGE_IMPORTS_INVENTORY
    if status == "absent":
        status, text = ratchet_base.file_at_base(root, LEGACY_DEVLOOP_STD_IMPORTS_INVENTORY)
        source = LEGACY_DEVLOOP_STD_IMPORTS_INVENTORY
    if status == "absent":
        return status, None, []
    if status == "unresolved" or text is None:
        return "unresolved", None, []
    entries, messages = load_devloop_forge_import_inventory_text(text, f"base:{source}")
    entries.update(FORGE_STRINGS_SPLIT_IMPORTS)
    return "present", entries, messages


def is_sanctioned_devloop_forge_validator_import(path: str, module: str) -> bool:
    return path == DEVLOOP_FORGE_VALIDATORS_FACADE and module in DEVLOOP_FORGE_VALIDATOR_MODULES


def same_module_import_count(inventory: set[tuple[str, str]], module: str) -> int:
    return sum(1 for _path, imported_module in inventory if imported_module == module)


def is_replacing_existing_devloop_forge_validator_import(
    item: tuple[str, str],
    current_inventory: set[tuple[str, str]],
    base_inventory: set[tuple[str, str]],
) -> bool:
    path, module = item
    return (
        is_sanctioned_devloop_forge_validator_import(path, module)
        and same_module_import_count(current_inventory, module) < same_module_import_count(base_inventory, module)
    )



def check_devloop_visibility(root: Path, violations: list[str], add) -> None:
    observed = load_visibility_allow(root / "libraries" / "devloop" / "fkst.toml")
    if observed != DEVLOOP_FAMILY:
        add(violations, "G-LIB-DEP", f"devloop visibility must list only {sorted(DEVLOOP_FAMILY)}; observed {sorted(observed)}")




def check_devloop_forge_import_inventory(root: Path, violations: list[str], read_text, rel, add, strip_lua_comments_and_strings, is_unmasked_range) -> None:
    devloop_forge_imports: set[tuple[str, str]] = set()
    for path in library_lua_files(root, "devloop"):
        rel_path = rel(root, path)
        for module, _line in require_literals(read_text(path), strip_lua_comments_and_strings, is_unmasked_range):
            if module.split(".")[0] == "forge":
                devloop_forge_imports.add((rel_path, module))
    inventory_path = root / DEVLOOP_FORGE_IMPORTS_INVENTORY
    if library_lua_files(root, "devloop") or inventory_path.exists():
        current_inventory, inventory_errors = load_devloop_forge_import_inventory(inventory_path)
        for message in inventory_errors:
            add(violations, "G-LIB-DEP", message)
        for path, module in sorted(devloop_forge_imports):
            if module in DEVLOOP_FORGE_VALIDATOR_MODULES and path != DEVLOOP_FORGE_VALIDATORS_FACADE:
                add(violations, "G-LIB-DEP", f"{path} imports {module}; use {DEVLOOP_FORGE_VALIDATORS_FACADE} instead")
        for item in sorted(devloop_forge_imports - current_inventory):
            path, module = item
            add(violations, "G-LIB-DEP", f"{path} imports {module} but is not listed in {DEVLOOP_FORGE_IMPORTS_INVENTORY}")
        for item in sorted(current_inventory - devloop_forge_imports):
            path, module = item
            add(violations, "G-LIB-DEP", f"{DEVLOOP_FORGE_IMPORTS_INVENTORY} lists stale import {path} {module}")
        base_status, base_inventory, base_errors = devloop_forge_imports_at_base(root)
        for message in base_errors:
            add(violations, "G-LIB-DEP", message)
        if base_status == "unresolved":
            add(violations, "G-LIB-DEP", f"cannot resolve dev base {DEVLOOP_FORGE_IMPORTS_INVENTORY} to enforce shrink-only ratchet")
        elif base_inventory is not None:
            for item in sorted(current_inventory - base_inventory):
                path, module = item
                if is_replacing_existing_devloop_forge_validator_import(item, current_inventory, base_inventory):
                    continue
                add(violations, "G-LIB-DEP", f"{DEVLOOP_FORGE_IMPORTS_INVENTORY} grows relative to dev: {path} {module}")


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
    check_devloop_visibility(root, violations, add)
    check_devloop_forge_import_inventory(root, violations, read_text, rel, add, strip_lua_comments_and_strings, is_unmasked_range)
