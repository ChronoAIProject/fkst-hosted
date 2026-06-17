#!/usr/bin/env python3
"""Dry-run allowlist migration slicer for code-owned ratchet parents."""

from __future__ import annotations

import argparse
import re
import sys
from dataclasses import dataclass
from pathlib import Path
from typing import Callable

import check_repo_gh_git_adapter as gh_git_adapter


TARGET_COUNT = 0
DEFAULT_SLICE_SIZE = 3
MAX_SLICE_SIZE = 10
VALID_PARENTS = ("891", "892")
FREE_FORM_PIPELINE_RE = re.compile(r"(?m)^\s*(?:function\s+pipeline\s*\(|pipeline\s*=\s*function\b)")


@dataclass(frozen=True)
class MigrationSpec:
    parent: str
    migration_kind: str
    allowlist_path: str
    title: str
    inventory_loader: Callable[[Path, "MigrationSpec"], list["InventorySite"]]


@dataclass(frozen=True)
class InventorySite:
    path: str
    line: int
    detail: str

    def site_ref(self) -> str:
        return f"{self.path}:{self.line}"


def repo_rel(root: Path, path: Path) -> str:
    packages = root / "packages"
    try:
        return "packages/" + path.relative_to(packages).as_posix()
    except ValueError:
        return path.relative_to(root).as_posix()


def read_text(path: Path) -> str:
    return path.read_text(encoding="utf-8")


def validated_repo_path(root: Path, relpath: str) -> Path:
    if relpath.startswith("/") or relpath == "" or "\x00" in relpath:
        raise ValueError(f"invalid repository path: {relpath}")
    parts = Path(relpath).parts
    if ".." in parts:
        raise ValueError(f"invalid repository path: {relpath}")
    path = root / relpath
    try:
        path.resolve().relative_to(root.resolve())
    except ValueError as exc:
        raise ValueError(f"repository path escapes root: {relpath}") from exc
    if not path.is_file():
        raise ValueError(f"repository path does not exist: {relpath}")
    return path


def line_for_offset(text: str, offset: int) -> int:
    return text.count("\n", 0, offset) + 1


def gh_git_head_locations(source: str) -> dict[str, int]:
    mask = gh_git_adapter.lua_code_mask(source)
    contexts = gh_git_adapter.lua_call_contexts(mask)
    literals = gh_git_adapter.lua_string_literals(source)
    literals_by_start = {literal.start: literal for literal in literals}
    locations: dict[str, int] = {}
    previous_literals: list[gh_git_adapter.LuaStringLiteral] = []
    for literal in literals:
        if gh_git_adapter.prior_literal_in_concat(source, literal, previous_literals):
            previous_literals.append(literal)
            continue
        head = gh_git_adapter.exec_argv_head_for_literal(source, mask, contexts, literal, literals_by_start)
        if head is None:
            command = gh_git_adapter.command_prefix_for_literal(source, mask, literal, literals_by_start)
            head = gh_git_adapter.normalized_command_head(command)
        if head is not None and not gh_git_adapter.is_excluded_literal(contexts, literal.start):
            locations.setdefault(head, line_for_offset(source, literal.start))
        previous_literals.append(literal)
    return locations


def load_gh_git_inventory(root: Path, spec: MigrationSpec) -> list[InventorySite]:
    allowlist = gh_git_adapter.load_allowlist(validated_repo_path(root, spec.allowlist_path))
    sources = gh_git_adapter.sources(root, root / "packages", read_text, repo_rel)
    sites: list[InventorySite] = []
    for relpath, heads in sorted(allowlist.items()):
        source_path = validated_repo_path(root, relpath)
        source = sources.get(relpath) or read_text(source_path)
        locations = gh_git_head_locations(source)
        for head in sorted(heads):
            line = locations.get(head)
            if line is None:
                raise ValueError(f"allowlist entry is not present in source: {relpath} -> {head}")
            sites.append(InventorySite(relpath, line, f"command_head: {head}"))
    return sites


def load_saga_inventory(root: Path, spec: MigrationSpec) -> list[InventorySite]:
    allowlist_path = validated_repo_path(root, spec.allowlist_path)
    sites: list[InventorySite] = []
    for raw in read_text(allowlist_path).splitlines():
        relpath = raw.strip()
        if not relpath or relpath.startswith("#"):
            continue
        source_path = validated_repo_path(root, relpath)
        source = read_text(source_path)
        masked = strip_lua_comments_and_strings(source)
        match = FREE_FORM_PIPELINE_RE.search(masked)
        if match is None:
            sites.append(InventorySite(relpath, 1, "stale_allowlist_entry"))
        else:
            sites.append(InventorySite(relpath, line_for_offset(masked, match.start()), "free_form_pipeline"))
    return sorted(sites, key=lambda site: (site.path, site.line, site.detail))


def strip_lua_comments_and_strings(text: str) -> str:
    return gh_git_adapter.lua_code_mask(text)


def specs() -> dict[str, MigrationSpec]:
    return {
        "891": MigrationSpec(
            parent="891",
            migration_kind="allowlist",
            allowlist_path="migration/gh-git-adapter.allowlist",
            title="gh/git ports adapter allowlist migration slice",
            inventory_loader=load_gh_git_inventory,
        ),
        "892": MigrationSpec(
            parent="892",
            migration_kind="allowlist",
            allowlist_path="migration/saga-handler.allowlist",
            title="saga handler allowlist migration slice",
            inventory_loader=load_saga_inventory,
        ),
    }


def render_child_issue(spec: MigrationSpec, inventory: list[InventorySite], slice_size: int) -> str:
    selected = inventory[:slice_size]
    lines = [
        f"# {spec.title}",
        "",
        "Dry-run child issue draft. No GitHub state was modified.",
        "",
        "## Ratchet",
        f"- parent_issue: #{spec.parent}",
        f"- migration_kind: {spec.migration_kind}",
        f"- allowlist_path: {spec.allowlist_path}",
        f"- current_count: {len(inventory)}",
        f"- target_count: {TARGET_COUNT}",
        f"- slice_size: {slice_size}",
        f"- selected_count: {len(selected)}",
        "",
        "## Exact Sites",
    ]
    if selected:
        for site in selected:
            lines.append(f"- {site.site_ref()} ({site.detail})")
    else:
        lines.append("- none")
    lines.extend(
        [
            "",
            "## Acceptance Criteria",
            "- Migrate only the exact sites listed above.",
            f"- Remove only those migrated entries from `{spec.allowlist_path}`.",
            f"- The allowlist count decreases by exactly {len(selected)}.",
            "- Behavior is preserved.",
            "- No broad cleanup, opportunistic refactors, or unrelated migration work.",
        ]
    )
    if not selected:
        lines.append("- No child issue is needed because the target count is already reached.")
    return "\n".join(lines) + "\n"


def parse_args(argv: list[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Print a deterministic dry-run child issue body for a code-owned allowlist ratchet parent.",
    )
    parser.add_argument("parent", choices=VALID_PARENTS, help="Code-owned parent issue selector.")
    parser.add_argument("--repo-root", default=Path(__file__).resolve().parents[1], type=Path)
    parser.add_argument("--slice-size", type=int, default=DEFAULT_SLICE_SIZE)
    return parser.parse_args(argv)


def main(argv: list[str] | None = None) -> int:
    args = parse_args(list(sys.argv[1:] if argv is None else argv))
    if args.slice_size < 1 or args.slice_size > MAX_SLICE_SIZE:
        print(f"error: --slice-size must be between 1 and {MAX_SLICE_SIZE}", file=sys.stderr)
        return 2
    root = args.repo_root.resolve()
    spec = specs()[args.parent]
    try:
        inventory = spec.inventory_loader(root, spec)
    except Exception as exc:
        print(f"error: ratchet inventory failed: {exc}", file=sys.stderr)
        return 1
    print(render_child_issue(spec, inventory, args.slice_size), end="")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
