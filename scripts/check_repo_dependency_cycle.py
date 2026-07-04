#!/usr/bin/env python3
"""Shrink-only ratchet for Lua require cycles among workspace library modules."""

from __future__ import annotations

from pathlib import Path
from typing import Callable

import check_repo_std_dependency_model
import ratchet_base


ALLOWLIST = "migration/dependency-cycle.allowlist"


def library_lua_files(root: Path) -> list[Path]:
    libraries = root / "libraries"
    if not libraries.exists():
        return []
    return sorted(path for path in libraries.rglob("*.lua") if path.is_file())


def module_name(root: Path, path: Path) -> str:
    rel = path.relative_to(root / "libraries").with_suffix("")
    return ".".join(rel.parts)


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


def dependency_graph(
    root: Path,
    read_text: Callable[[Path], str],
    strip_lua_comments_and_strings: Callable[[str], str],
    is_unmasked_range: Callable[[str, str, int, int], bool],
) -> dict[str, set[str]]:
    files = library_lua_files(root)
    modules = {module_name(root, path): path for path in files}
    graph = {module: set() for module in modules}
    for module, path in modules.items():
        for required, _line in require_literals(read_text(path), strip_lua_comments_and_strings, is_unmasked_range):
            if required in modules:
                graph[module].add(required)
    return graph


def strongly_connected_components(graph: dict[str, set[str]]) -> list[set[str]]:
    index = 0
    stack: list[str] = []
    on_stack: set[str] = set()
    indexes: dict[str, int] = {}
    lowlinks: dict[str, int] = {}
    components: list[set[str]] = []

    def visit(node: str) -> None:
        nonlocal index
        indexes[node] = index
        lowlinks[node] = index
        index += 1
        stack.append(node)
        on_stack.add(node)

        for target in sorted(graph[node]):
            if target not in indexes:
                visit(target)
                lowlinks[node] = min(lowlinks[node], lowlinks[target])
            elif target in on_stack:
                lowlinks[node] = min(lowlinks[node], indexes[target])

        if lowlinks[node] != indexes[node]:
            return
        component: set[str] = set()
        while True:
            popped = stack.pop()
            on_stack.remove(popped)
            component.add(popped)
            if popped == node:
                break
        components.append(component)

    for node in sorted(graph):
        if node not in indexes:
            visit(node)
    return components


def canonical_cycle(component: set[str]) -> str:
    return " <-> ".join(sorted(component))


def simple_cycles_in_component(graph: dict[str, set[str]], component: set[str]) -> set[str]:
    ordered = sorted(component)
    rank = {node: index for index, node in enumerate(ordered)}
    cycles: set[str] = set()

    def visit(start: str, node: str, path: list[str], seen: set[str]) -> None:
        for target in sorted(graph[node] & component):
            if target == start:
                cycles.add(canonical_cycle(set(path)))
                continue
            if target in seen or rank[target] < rank[start]:
                continue
            visit(start, target, path + [target], seen | {target})

    for start in ordered:
        visit(start, start, [start], {start})
    return cycles


def cycles_from_graph(graph: dict[str, set[str]]) -> set[str]:
    cycles: set[str] = set()
    for component in strongly_connected_components(graph):
        if len(component) > 1:
            cycles.update(simple_cycles_in_component(graph, component))
            continue
        node = next(iter(component))
        if node in graph[node]:
            cycles.add(node)
    return cycles


def current_cycles(
    root: Path,
    read_text: Callable[[Path], str],
    strip_lua_comments_and_strings: Callable[[str], str],
    is_unmasked_range: Callable[[str, str, int, int], bool],
) -> set[str]:
    return cycles_from_graph(
        dependency_graph(root, read_text, strip_lua_comments_and_strings, is_unmasked_range)
    )


def parse_allowlist_lines(lines: list[str]) -> set[str]:
    entries: set[str] = set()
    for raw in lines:
        line = raw.split("#", 1)[0].strip()
        if not line:
            continue
        modules = line.split(" <-> ")
        if any(not module for module in modules) or modules != sorted(modules):
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


def ratchet_messages(
    current: set[str],
    allowlist: set[str],
    base_allowlist: set[str] | None = None,
) -> list[str]:
    messages = [
        f"{cycle} is a library require cycle and is not in {ALLOWLIST}; break the cycle or shrink the allowlist only"
        for cycle in sorted(current - allowlist)
    ]
    messages.extend(
        f"{cycle} is stale in {ALLOWLIST}; prune the stale entry"
        for cycle in sorted(allowlist - current)
    )
    if base_allowlist is not None:
        messages.extend(
            f"{cycle} grows {ALLOWLIST} relative to dev; break the new library require cycle instead"
            for cycle in sorted(allowlist - base_allowlist)
        )
    return messages


def messages(
    root: Path,
    read_text: Callable[[Path], str],
    strip_lua_comments_and_strings: Callable[[str], str],
    is_unmasked_range: Callable[[str, str, int, int], bool],
    allowlist_dir: Path | None = None,
    enforce_base: bool = True,
) -> list[str]:
    current = current_cycles(root, read_text, strip_lua_comments_and_strings, is_unmasked_range)
    allowlist_path = root / ALLOWLIST if allowlist_dir is None else allowlist_dir / Path(ALLOWLIST).name
    allowlist = load_allowlist(allowlist_path)
    base_status, base_allowlist = allowlist_at_dev_base(root) if enforce_base else ("absent", None)
    result: list[str] = []
    if base_status == "unresolved":
        result.append("cannot resolve dev base allowlist to enforce shrink-only dependency-cycle ratchet; ensure CI provides the dev ref")
    result.extend(ratchet_messages(current, allowlist, base_allowlist))
    return result
