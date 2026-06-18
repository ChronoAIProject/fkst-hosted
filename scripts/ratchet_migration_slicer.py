#!/usr/bin/env python3
"""Dry-run allowlist migration slicer for code-owned ratchet parents."""

from __future__ import annotations

import argparse
import hashlib
import json
import re
import sys
from dataclasses import dataclass
from pathlib import Path
from typing import Callable

import check_repo_dedup as code_dedup
import check_repo_gh_git_adapter as gh_git_adapter


TARGET_COUNT = 0
DEFAULT_SLICE_SIZE = 3
MAX_SLICE_SIZE = 10
SCHEMA = "fkst.ratchet-slice.v1"
VALID_RATCHETS = ("gh-git-adapter", "saga-handler", "code-dedup")
RATCHET_ALIASES = {
    "891": "gh-git-adapter",
    "892": "saga-handler",
}
FREE_FORM_PIPELINE_RE = re.compile(r"(?m)^\s*(?:function\s+pipeline\s*\(|pipeline\s*=\s*function\b)")


@dataclass(frozen=True)
class MigrationSpec:
    parent: str
    ratchet: str
    migration_kind: str
    allowlist_path: str
    title: str
    reference_shape: str
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


def load_code_dedup_inventory(root: Path, spec: MigrationSpec) -> list[InventorySite]:
    allowlist_path = validated_repo_path(root, spec.allowlist_path)
    sites: list[InventorySite] = []
    for entry in sorted(code_dedup.load_allowlist(allowlist_path)):
        for relpath in entry.files:
            source_path = validated_repo_path(root, relpath)
            source = read_text(source_path)
            line = line_for_function_basename(source, entry.name)
            detail = f"duplicate_function: {entry.name} {entry.body_hash}"
            sites.append(InventorySite(relpath, line or 1, detail))
    return sorted(sites, key=lambda site: (site.path, site.line, site.detail))


def line_for_function_basename(source: str, basename: str) -> int | None:
    code = code_dedup.code_without_comments_and_strings(source)
    for offset, line in enumerate(code.splitlines(), start=1):
        match = code_dedup.FUNCTION_RE.match(line)
        if match is not None and code_dedup.function_basename(match.group("name")) == basename:
            return offset
    return None


def strip_lua_comments_and_strings(text: str) -> str:
    return gh_git_adapter.lua_code_mask(text)


def specs() -> dict[str, MigrationSpec]:
    return {
        "gh-git-adapter": MigrationSpec(
            parent="891",
            ratchet="gh-git-adapter",
            migration_kind="allowlist",
            allowlist_path="migration/gh-git-adapter.allowlist",
            title="gh/git ports adapter allowlist migration slice",
            reference_shape="Migrate raw gh/git command construction behind std.github/std.git adapter operations.",
            inventory_loader=load_gh_git_inventory,
        ),
        "saga-handler": MigrationSpec(
            parent="979",
            ratchet="saga-handler",
            migration_kind="allowlist",
            allowlist_path="migration/saga-handler.allowlist",
            title="saga handler allowlist migration slice",
            reference_shape="Use the existing std.saga.department(spec, handlers) shape from migrated departments.",
            inventory_loader=load_saga_inventory,
        ),
        "code-dedup": MigrationSpec(
            parent="1002",
            ratchet="code-dedup",
            migration_kind="allowlist",
            allowlist_path="migration/code-dedup.allowlist",
            title="code dedup allowlist migration slice",
            reference_shape="Hoist the byte-identical production Lua function body to an existing shared module such as std.*, then call the shared helper from each site.",
            inventory_loader=load_code_dedup_inventory,
        ),
    }


def markdown_code(value: str) -> str:
    text = str(value).replace("`", "\\`")
    return f"`{text}`"


def selected_sites(inventory: list[InventorySite], slice_size: int) -> list[InventorySite]:
    return inventory[:slice_size]


def site_records(inventory: list[InventorySite], slice_size: int) -> list[dict[str, object]]:
    return [
        {
            "path": site.path,
            "line": site.line,
            "detail": site.detail,
            "site_ref": site.site_ref(),
        }
        for site in selected_sites(inventory, slice_size)
    ]


def sites_fingerprint(sites: list[dict[str, object]]) -> str:
    encoded = json.dumps(sites, sort_keys=True, separators=(",", ":"), ensure_ascii=False)
    return hashlib.sha256(encoded.encode("utf-8")).hexdigest()[:16]


def slice_document(spec: MigrationSpec, inventory: list[InventorySite], slice_size: int) -> dict[str, object]:
    sites = site_records(inventory, slice_size)
    fingerprint = sites_fingerprint(sites)
    return {
        "schema": SCHEMA,
        "ratchet": spec.ratchet,
        "parent_issue": int(spec.parent),
        "migration_kind": spec.migration_kind,
        "allowlist_path": spec.allowlist_path,
        "title": spec.title,
        "reference_shape": spec.reference_shape,
        "current_count": len(inventory),
        "target_count": TARGET_COUNT,
        "slice_size": slice_size,
        "selected_count": len(sites),
        "sites_fingerprint": fingerprint,
        "dedup_key": f"{spec.ratchet}/slice/{fingerprint}",
        "sites": sites,
    }


def render_child_issue(spec: MigrationSpec, inventory: list[InventorySite], slice_size: int) -> str:
    doc = slice_document(spec, inventory, slice_size)
    selected = selected_sites(inventory, slice_size)
    lines = [
        f"# {spec.title}",
        "",
        "Dry-run child issue draft. No GitHub state was modified.",
        "",
        "## Ratchet",
        f"- parent_issue: #{spec.parent}",
        f"- ratchet: {markdown_code(spec.ratchet)}",
        f"- migration_kind: {markdown_code(spec.migration_kind)}",
        f"- allowlist_path: {markdown_code(spec.allowlist_path)}",
        f"- current_count: {len(inventory)}",
        f"- target_count: {TARGET_COUNT}",
        f"- slice_size: {slice_size}",
        f"- selected_count: {len(selected)}",
        f"- sites_fingerprint: {markdown_code(str(doc['sites_fingerprint']))}",
        f"- dedup_key: {markdown_code(str(doc['dedup_key']))}",
        "",
        "## Reference Shape",
        spec.reference_shape,
        "",
        "## Exact Sites",
    ]
    if selected:
        for site in selected:
            lines.append(f"- {markdown_code(site.site_ref())} ({markdown_code(site.detail)})")
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
            "- `scripts/run.sh test` exits 0.",
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
    parser.add_argument("ratchet", choices=(*VALID_RATCHETS, *RATCHET_ALIASES.keys()), help="Code-owned ratchet selector.")
    parser.add_argument("--repo-root", default=Path(__file__).resolve().parents[1], type=Path)
    parser.add_argument("--slice-size", type=int, default=DEFAULT_SLICE_SIZE)
    parser.add_argument("--json", action="store_true", help="Emit the stable machine-readable slice schema.")
    return parser.parse_args(argv)


def main(argv: list[str] | None = None) -> int:
    args = parse_args(list(sys.argv[1:] if argv is None else argv))
    if args.slice_size < 1 or args.slice_size > MAX_SLICE_SIZE:
        print(f"error: --slice-size must be between 1 and {MAX_SLICE_SIZE}", file=sys.stderr)
        return 2
    root = args.repo_root.resolve()
    ratchet = RATCHET_ALIASES.get(args.ratchet, args.ratchet)
    spec = specs()[ratchet]
    try:
        inventory = spec.inventory_loader(root, spec)
    except Exception as exc:
        print(f"error: ratchet inventory failed: {exc}", file=sys.stderr)
        return 1
    if args.json:
        print(json.dumps(slice_document(spec, inventory, args.slice_size), sort_keys=True, ensure_ascii=False))
    else:
        print(render_child_issue(spec, inventory, args.slice_size), end="")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
