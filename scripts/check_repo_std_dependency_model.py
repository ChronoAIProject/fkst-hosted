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
FORGE_STRINGS_SPLIT_IMPORTS = {
    ("libraries/devloop/parsers/misc.lua", "forge.strings"),
}
LIBRARIES = ("contract", "workflow", "testkit", "forge", "devloop")
CONTRACT_MODULES = {"error_facts", "payload", "source_ref", "strings"}
DEVLOOP_FAMILY = {
    "fkst-substrate-ref-maintainer",
    "github-devloop",
    "github-devloop-decompose",
    "github-devloop-intake",
    "github-devloop-integration",
    "github-devloop-pr",
}
WORKFLOW_FORBIDDEN_STRINGS = (
    "state:v1",
    "pr-delegation:v1",
    "pr-comment-stream",
    "implement-attempt",
    "devloop_fixing",
    "github-devloop",
    "fkst-dev:",
    "forge.github",
    "forge.git",
    "std.github",
    "std.git",
)
WORKFLOW_FORBIDDEN_RAW_COMMAND_RE = re.compile(r"(?<![A-Za-z0-9_.-])(?:gh|git)(?:\s|['\"]|$)")


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


def package_lua_files(package_root: Path) -> Iterable[Path]:
    for path in sorted(package_root.rglob("*.lua")):
        if path.is_file():
            yield path


def library_module_name(root: Path, path: Path, library: str) -> str:
    relative = path.relative_to(root / "libraries" / library)
    if relative.name == "init.lua":
        parts = relative.parent.parts
    else:
        parts = relative.with_suffix("").parts
    return ".".join((library, *parts)) if parts else library


def module_path(root: Path, module: str) -> Path | None:
    top, _, rest = module.partition(".")
    if top not in LIBRARIES or rest == "":
        return None
    return root / "libraries" / top / Path(*rest.split("."))


def module_exists(root: Path, module: str) -> bool:
    path = module_path(root, module)
    if path is None:
        return True
    return path.with_suffix(".lua").is_file() or (path / "init.lua").is_file()


def modules_for_path(root: Path, path: Path) -> set[str]:
    modules: set[str] = set()
    for library in LIBRARIES:
        try:
            path.relative_to(root / "libraries" / library)
        except ValueError:
            continue
        modules.add(library)
        if library == "devloop":
            modules.update({"contract", "workflow", "forge"})
        return modules
    try:
        package = path.relative_to(root / "packages").parts[0]
    except (ValueError, IndexError):
        return set()
    deps = package_lib_deps(root / "packages" / package / "fkst.toml")
    return deps | {package}


def package_lib_deps(path: Path) -> set[str]:
    if not path.exists():
        return set()
    text = path.read_text(encoding="utf-8")
    match = re.search(r"(?ms)^\[lib_deps\]\s*\n\s*libraries\s*=\s*\[(?P<body>.*?)\]", text)
    if match is None:
        return set()
    return set(re.findall(r"[\"']([A-Za-z0-9_.-]+)[\"']", match.group("body")))


def manifest_value(text: str, key: str) -> str | None:
    match = re.search(r"(?m)^" + re.escape(key) + r"\s*=\s*[\"']([^\"']+)[\"']", text)
    return None if match is None else match.group(1)


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


def check_contract_surface(root: Path, violations: list[str], add) -> None:
    contract_root = root / "libraries" / "contract"
    actual = {path.with_suffix("").name for path in contract_root.glob("*.lua") if path.is_file()}
    if actual != CONTRACT_MODULES:
        add(violations, "G-LIB-DEP", f"contract modules must be exactly {sorted(CONTRACT_MODULES)}; observed {sorted(actual)}")
    manifest = contract_root / "fkst.toml"
    if not manifest.exists():
        add(violations, "G-LIB-DEP", "libraries/contract/fkst.toml is required")
        return
    text = manifest.read_text(encoding="utf-8")
    if re.search(r"(?ms)^\[lib_deps\]\s*\n\s*libraries\s*=\s*\[\s*\]", text) is None:
        add(violations, "G-LIB-DEP", "contract must declare zero outgoing lib_deps")
    if re.search(r"(?ms)^\[visibility\]\s*\n\s*public\s*=\s*true", text) is None:
        add(violations, "G-LIB-DEP", "contract must be the public publishable library")
    for library in ("workflow", "testkit", "forge", "devloop"):
        lib_manifest = root / "libraries" / library / "fkst.toml"
        if not lib_manifest.exists():
            continue
        if re.search(r"(?ms)^\[visibility\]\s*\n\s*public\s*=\s*true", lib_manifest.read_text(encoding="utf-8")) is not None:
            add(violations, "G-LIB-DEP", f"{library} must not be public/publishable")


def check_devloop_visibility(root: Path, violations: list[str], add) -> None:
    observed = load_visibility_allow(root / "libraries" / "devloop" / "fkst.toml")
    if observed != DEVLOOP_FAMILY:
        add(violations, "G-LIB-DEP", f"devloop visibility must list only {sorted(DEVLOOP_FAMILY)}; observed {sorted(observed)}")


def devloop_family(root: Path) -> set[str]:
    observed = load_visibility_allow(root / "libraries" / "devloop" / "fkst.toml")
    return observed if observed else DEVLOOP_FAMILY


def check_workflow_policy(root: Path, violations: list[str], read_text, rel, add) -> None:
    for path in library_lua_files(root, "workflow"):
        stripped = read_text(path)
        for line_number, line in enumerate(stripped.splitlines(), start=1):
            for needle in WORKFLOW_FORBIDDEN_STRINGS:
                if needle in line:
                    add(violations, "G-WORKFLOW-POLICY", f"{rel(root, path)}:{line_number} contains product/forge policy string {needle!r}")
            if WORKFLOW_FORBIDDEN_RAW_COMMAND_RE.search(line) is not None:
                add(violations, "G-WORKFLOW-POLICY", f"{rel(root, path)}:{line_number} contains raw gh/git command text")


def check_require_edges(root: Path, violations: list[str], warnings: list[str], packages, read_text, rel, add, strip_lua_comments_and_strings, is_unmasked_range) -> None:
    package_usage: dict[str, set[str]] = {package.name: set() for package in packages}
    library_edges: dict[str, set[str]] = {library: set() for library in LIBRARIES}
    devloop_forge_imports: set[tuple[str, str]] = set()
    devloop_visible_packages = devloop_family(root)
    allowed = {
        "contract": {"contract"},
        "workflow": {"workflow", "contract"},
        "testkit": {"testkit", "contract", "workflow"},
        "forge": {"forge", "contract"},
        "devloop": {"devloop", "contract", "workflow", "forge"},
    }
    for library in LIBRARIES:
        for path in library_lua_files(root, library):
            rel_path = rel(root, path)
            source_module = library_module_name(root, path, library)
            for module, line in require_literals(read_text(path), strip_lua_comments_and_strings, is_unmasked_range):
                top = module.split(".")[0]
                if top in LIBRARIES:
                    if not module_exists(root, module):
                        add(violations, "G-LIB-DEP", f"{rel_path}:{line} requires unresolved module {module!r}")
                    if top not in allowed[library]:
                        add(violations, "G-LIB-DEP", f"{rel_path}:{line} {library} must not require {module!r}")
                    else:
                        library_edges[library].add(f"{source_module}->{module}")
                    if library == "devloop" and top == "forge":
                        devloop_forge_imports.add((rel_path, module))
                    continue
                if module in LIBRARIES:
                    continue
                if library in {"contract", "workflow", "testkit", "forge"}:
                    add(violations, "G-LIB-DEP", f'{library} module {rel_path}:{line} requires non-library module "{module}"')
    for package in packages:
        deps = package_lib_deps(package / "fkst.toml")
        if "devloop" in deps and package.name not in devloop_visible_packages:
            add(violations, "G-LIB-DEP", f"package {package.name} must not declare lib_dep 'devloop'")
        for path in package_lua_files(package):
            for module, line in require_literals(read_text(path), strip_lua_comments_and_strings, is_unmasked_range):
                top = module.split(".")[0]
                if top not in LIBRARIES:
                    continue
                package_usage[package.name].add(module)
                if not module_exists(root, module):
                    add(violations, "G-LIB-DEP", f"{rel(root, path)}:{line} requires unresolved module {module!r}")
                if top not in deps:
                    add(violations, "G-LIB-DEP", f"{rel(root, path)}:{line} package {package.name} requires {module!r} but fkst.toml does not declare lib_dep {top!r}")
                if top == "devloop" and package.name not in devloop_visible_packages:
                    add(violations, "G-LIB-DEP", f"{rel(root, path)}:{line} package {package.name} must not require {module!r}")
    inventory_path = root / DEVLOOP_FORGE_IMPORTS_INVENTORY
    if library_lua_files(root, "devloop") or inventory_path.exists():
        current_inventory, inventory_errors = load_devloop_forge_import_inventory(inventory_path)
        for message in inventory_errors:
            add(violations, "G-LIB-DEP", message)
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
                add(violations, "G-LIB-DEP", f"{DEVLOOP_FORGE_IMPORTS_INVENTORY} grows relative to dev: {path} {module}")
    for package_name, modules in sorted(package_usage.items()):
        if modules:
            add(warnings, "G-LIB-DEP", f"package {package_name} uses {', '.join(sorted(modules))}")
    for library, edges in sorted(library_edges.items()):
        if edges:
            add(warnings, "G-LIB-DEP", f"library {library} edges {', '.join(sorted(edges))}")


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
    check_contract_surface(root, violations, add)
    check_devloop_visibility(root, violations, add)
    check_workflow_policy(root, violations, read_text, rel, add)
    check_require_edges(root, violations, warnings, packages, read_text, rel, add, strip_lua_comments_and_strings, is_unmasked_range)
