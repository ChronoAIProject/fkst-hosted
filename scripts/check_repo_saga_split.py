#!/usr/bin/env python3
"""Migration ratchet for the github-devloop issue/PR saga split.

This scanner is a migration ratchet, not the structural boundary. It catches
direct literal PR-phase writes through state-marker authority sinks and
issue-side code promoting linked PR state as issue authority. Label hints and
PR phases passed through variables remain residual data-flow/call-site debt
until the Step 2 package boundary makes the bypass structurally impossible.
"""

from __future__ import annotations

import json
import re
from dataclasses import dataclass
from pathlib import Path

import ratchet_base


MANIFEST = "migration/github-devloop-saga-split.inventory"
ALLOWLIST = "migration/github-devloop-saga-split-authority.allowlist"
SPEC_REF = "docs/superpowers/specs/2026-06-20-issue-pr-saga-split-design.md"
CONTRACT = "libraries/devloop/restart/issue/pr_partition_contract.lua"
OWNERS = {"issue", "pr", "shared", "integration", "cross-cutting", "intake"}
CALL_SCAN_MAX_CHARS = 12000
CALL_SCAN_MAX_LINES = 120

PR_PHASE_BLOCK_RE = re.compile(r"\blocal\s+PR_PHASE_STATES\s*=\s*\{(?P<body>.*?)\}", re.DOTALL)
LUA_STRING_RE = re.compile(r"(?P<quote>[\"'])(?P<value>[^\"']+)(?P=quote)")
STATE_WRITE_HELPERS = {
    "state_marker": ("state-marker", 2),
}
STATE_WRITE_CALL_RE = re.compile(
    r"(?<![\w.])(?:[A-Za-z_][\w]*\.)?(?P<helper>"
    + "|".join(sorted((re.escape(helper) for helper in STATE_WRITE_HELPERS), key=len, reverse=True))
    + r")\s*\("
)
LINKED_STATE_RE = re.compile(
    r"\b(?:issue_authoritative_linked_state|linked_snapshot_issue_state|current_linked_entity_state|linked_entity_snapshot)\b"
    r"|snapshot\.state\s*=\s*M\.current_entity_state\s*\(\s*snapshot\.comments"
    r"|copy_comments\s*\(\s*snapshot\.comments\s*,\s*current_pr\.comments\s*\)"
)


@dataclass(frozen=True, order=True)
class InventoryEntry:
    path: str
    owner: str
    reason: str


@dataclass(frozen=True, order=True)
class LeakSite:
    path: str
    kind: str
    token: str
    line: int

    @classmethod
    def parse(cls, line: str) -> "LeakSite":
        parts = line.split("|")
        if len(parts) < 6:
            raise ValueError(f"invalid {ALLOWLIST} line: {line}")
        path, kind, token, line_part, spec, why = parts[:6]
        if not (
            path.startswith("packages/github-devloop/")
            or path.startswith("packages/github-devloop-intake/")
            or path.startswith("packages/github-devloop-intake-default/")
            or path.startswith("packages/github-devloop-ops/")
            or path.startswith("packages/github-devloop-pr/")
            or path.startswith("packages/github-devloop-integration/")
            or path.startswith("libraries/devloop/")
        ) or not path.endswith(".lua"):
            raise ValueError(f"invalid {ALLOWLIST} path: {line}")
        if kind not in {"state-marker", "linked-state-promotion"}:
            raise ValueError(f"invalid {ALLOWLIST} kind: {line}")
        if spec != f"spec={SPEC_REF}":
            raise ValueError(f"invalid {ALLOWLIST} spec ref: {line}")
        if not why.startswith("why=") or why == "why=":
            raise ValueError(f"invalid {ALLOWLIST} WHY: {line}")
        if not line_part.startswith("line="):
            raise ValueError(f"invalid {ALLOWLIST} line number: {line}")
        line_number = int(line_part.removeprefix("line="))
        return cls(path=path, kind=kind, token=token, line=line_number)

    def key(self) -> tuple[str, str, str]:
        return self.path, self.kind, self.token, str(self.line)

    def allowlist_line(self, why: str) -> str:
        return "|".join((
            self.path,
            self.kind,
            self.token,
            f"line={self.line}",
            f"spec={SPEC_REF}",
            f"why={why}",
        ))

    def label(self) -> str:
        return f"{self.path}:{self.line} {self.kind} {self.token}"


def expected_paths(root: Path) -> set[str]:
    paths: set[str] = set()
    for package in (
        "github-devloop",
        "github-devloop-intake",
        "github-devloop-intake-default",
        "github-devloop-ops",
        "github-devloop-pr",
        "github-devloop-integration",
    ):
        base = root / "packages" / package
        paths.update(
            path.relative_to(root).as_posix()
            for path in sorted((base / "departments").glob("*/main.lua"))
            if path.is_file()
        )
        paths.update(
            path.relative_to(root).as_posix()
            for path in sorted((base / "core").rglob("*.lua"))
            if path.is_file()
        )
    devloop = root / "libraries" / "devloop"
    paths.update(
        path.relative_to(root).as_posix()
        for path in sorted(devloop.glob("*.lua"))
        if path.is_file()
    )
    for directory in sorted([devloop / "restart"]):
        if not directory.is_dir():
            continue
        paths.update(
            path.relative_to(root).as_posix()
            for path in sorted(directory.rglob("*.lua"))
            if path.is_file()
        )
    return paths


def load_manifest(path: Path) -> tuple[list[InventoryEntry], list[str]]:
    entries: list[InventoryEntry] = []
    messages: list[str] = []
    if not path.exists():
        return entries, [f"manifest-missing: {MANIFEST} is required"]
    for number, raw in enumerate(path.read_text(encoding="utf-8").splitlines(), start=1):
        stripped = raw.strip()
        if stripped == "" or stripped.startswith("#"):
            continue
        try:
            doc = json.loads(stripped)
        except json.JSONDecodeError as exc:
            messages.append(f"manifest-invalid-json: {MANIFEST}:{number}: {exc.msg}")
            continue
        entry = InventoryEntry(
            path=str(doc.get("path", "")),
            owner=str(doc.get("owner", "")),
            reason=str(doc.get("reason", "")),
        )
        if entry.owner not in OWNERS:
            messages.append(f"manifest-invalid-owner: {entry.path} owner={entry.owner}")
        if not entry.reason:
            messages.append(f"manifest-missing-reason: {entry.path}")
        entries.append(entry)
    return entries, messages


def manifest_messages(root: Path, entries: list[InventoryEntry]) -> list[str]:
    messages: list[str] = []
    expected = expected_paths(root)
    by_path: dict[str, list[InventoryEntry]] = {}
    for entry in entries:
        by_path.setdefault(entry.path, []).append(entry)
    for path in sorted(expected - set(by_path)):
        messages.append(f"manifest-missing-entry: {path} is not classified in {MANIFEST}")
    for path in sorted(set(by_path) - expected):
        messages.append(f"manifest-stale-entry: {path} is classified in {MANIFEST} but no longer exists")
    for path, duplicates in sorted(by_path.items()):
        if len(duplicates) > 1:
            messages.append(f"manifest-duplicate-entry: {path} appears {len(duplicates)} times in {MANIFEST}")
    return messages


def inventory_by_path(entries: list[InventoryEntry]) -> dict[str, InventoryEntry]:
    result: dict[str, InventoryEntry] = {}
    for entry in entries:
        if entry.path not in result:
            result[entry.path] = entry
    return result


def load_pr_phase_states(root: Path) -> set[str]:
    contract_path = root / CONTRACT
    try:
        text = contract_path.read_text(encoding="utf-8")
    except FileNotFoundError as exc:
        raise ValueError(f"contract-missing: {CONTRACT}") from exc
    match = PR_PHASE_BLOCK_RE.search(text)
    if match is None:
        raise ValueError(f"contract-malformed: missing PR_PHASE_STATES literal in {CONTRACT}")
    states = {m.group("value") for m in LUA_STRING_RE.finditer(_strip_lua_comments(match.group("body")))}
    if not states:
        raise ValueError(f"contract-malformed: empty PR_PHASE_STATES literal in {CONTRACT}")
    return states


def _strip_lua_line_comment(line: str) -> str:
    quote: str | None = None
    escaped = False
    index = 0
    while index < len(line):
        char = line[index]
        if quote is not None:
            if escaped:
                escaped = False
            elif char == "\\":
                escaped = True
            elif char == quote:
                quote = None
            index += 1
            continue
        if char in {"'", '"'}:
            quote = char
            index += 1
            continue
        if line.startswith("--", index):
            return line[:index]
        index += 1
    return line


def _strip_lua_comments(text: str) -> str:
    return "\n".join(_strip_lua_line_comment(line) for line in text.splitlines())


def _line_number(text: str, index: int) -> int:
    return text.count("\n", 0, index) + 1


def _call_end(text: str, open_paren: int) -> int | None:
    quote: str | None = None
    escaped = False
    depth = 0
    line_count = 0
    limit = min(len(text), open_paren + CALL_SCAN_MAX_CHARS)
    index = open_paren
    while index < limit:
        char = text[index]
        if char == "\n":
            line_count += 1
            if line_count > CALL_SCAN_MAX_LINES:
                return None
        if quote is not None:
            if escaped:
                escaped = False
            elif char == "\\":
                escaped = True
            elif char == quote:
                quote = None
            index += 1
            continue
        if char in {"'", '"'}:
            quote = char
            index += 1
            continue
        if char == "(":
            depth += 1
        elif char == ")":
            depth -= 1
            if depth == 0:
                return index + 1
        index += 1
    return None


def _call_arguments(call_text: str) -> list[tuple[str, int]]:
    open_paren = call_text.find("(")
    close_paren = call_text.rfind(")")
    if open_paren < 0 or close_paren <= open_paren:
        return []
    args_text = call_text[open_paren + 1:close_paren]
    args: list[tuple[str, int]] = []
    quote: str | None = None
    escaped = False
    depth = 0
    start = 0
    index = 0
    while index < len(args_text):
        char = args_text[index]
        if quote is not None:
            if escaped:
                escaped = False
            elif char == "\\":
                escaped = True
            elif char == quote:
                quote = None
            index += 1
            continue
        if char in {"'", '"'}:
            quote = char
            index += 1
            continue
        if char in "({[":
            depth += 1
        elif char in ")}]":
            depth -= 1
        elif char == "," and depth == 0:
            args.append((args_text[start:index], open_paren + 1 + start))
            start = index + 1
        index += 1
    args.append((args_text[start:], open_paren + 1 + start))
    return args


def _state_write_leaks(path: str, text: str, pr_phase_states: set[str]) -> set[LeakSite]:
    leaks: set[LeakSite] = set()
    clean = _strip_lua_comments(text)
    for call in STATE_WRITE_CALL_RE.finditer(clean):
        helper = call.group("helper")
        open_paren = call.end() - 1
        end = _call_end(clean, open_paren)
        if end is None:
            continue
        call_text = clean[call.start():end]
        kind, state_arg_index = STATE_WRITE_HELPERS[helper]
        args = _call_arguments(call_text)
        if len(args) < state_arg_index:
            continue
        state_arg, state_arg_start = args[state_arg_index - 1]
        for literal in LUA_STRING_RE.finditer(state_arg):
            state = literal.group("value")
            if state in pr_phase_states:
                if _is_allowed_issue_to_pr_boundary_seed(path, clean, call.start(), state):
                    continue
                leaks.add(LeakSite(
                    path=path,
                    kind=kind,
                    token=state,
                    line=_line_number(clean, call.start() + state_arg_start + literal.start()),
                ))
    return leaks


def _enclosing_function_body(text: str, index: int) -> str:
    function_start = text.rfind("local function ", 0, index)
    if function_start < 0:
        function_start = text.rfind("function ", 0, index)
    if function_start < 0:
        return ""
    next_function = text.find("\nlocal function ", function_start + 1)
    if next_function < 0:
        next_function = text.find("\nfunction ", function_start + 1)
    if next_function < 0:
        next_function = len(text)
    return text[function_start:next_function]


def _is_allowed_issue_to_pr_boundary_seed(path: str, text: str, index: int, state: str) -> bool:
    if path != "packages/github-devloop/core/pr_delegation.lua" or state != "pr-open":
        return False
    body = _enclosing_function_body(text, index)
    return (
        "build_pr_open_comment_request" in body
        and "pr_origin_marker" in body
        and "state_marker" in body
    )


def source_leaks(path: str, owner: str, text: str, pr_phase_states: set[str]) -> set[LeakSite]:
    if owner == "pr":
        return set()
    leaks: set[LeakSite] = set()
    leaks.update(_state_write_leaks(path, text, pr_phase_states))
    for line_number, raw in enumerate(text.splitlines(), start=1):
        line = _strip_lua_line_comment(raw)
        if LINKED_STATE_RE.search(line) is not None:
            leaks.add(LeakSite(path, "linked-state-promotion", "linked-pr-comments", line_number))
    return leaks


def current_leaks(root: Path, entries: list[InventoryEntry], pr_phase_states: set[str]) -> set[LeakSite]:
    inventory = inventory_by_path(entries)
    leaks: set[LeakSite] = set()
    for path, entry in sorted(inventory.items()):
        if entry.owner == "pr":
            continue
        source_path = root / path
        if source_path.exists():
            leaks.update(source_leaks(path, entry.owner, source_path.read_text(encoding="utf-8"), pr_phase_states))
    return leaks


def load_allowlist(path: Path) -> set[LeakSite]:
    if not path.exists():
        return set()
    entries: set[LeakSite] = set()
    for raw in path.read_text(encoding="utf-8").splitlines():
        stripped = raw.strip()
        if stripped == "" or stripped.startswith("#"):
            continue
        entries.add(LeakSite.parse(stripped))
    return entries


def covered_by_allowlist(site: LeakSite, allowlist: set[LeakSite]) -> bool:
    return any(entry.key() == site.key() for entry in allowlist)


def ratchet_messages(
    leaks: set[LeakSite],
    allowlist: set[LeakSite],
    base_allowlist: set[LeakSite] | None = None,
) -> list[str]:
    messages: list[str] = []
    for site in sorted(leaks):
        if not covered_by_allowlist(site, allowlist):
            messages.append(f"leak-new: {site.label()} writes or parses PR-phase authority from a non-PR owner")
    for entry in sorted(allowlist):
        if not any(site.key() == entry.key() for site in leaks):
            messages.append(f"allowlist-stale: {entry.label()} no longer matches saga-split authority debt")
    if base_allowlist is not None:
        for entry in sorted(allowlist):
            if not covered_by_allowlist(entry, base_allowlist):
                messages.append(f"allowlist-growth: {entry.label()} grows {ALLOWLIST} relative to dev")
    return messages


def allowlist_at_dev_base(root: Path) -> tuple[str, set[LeakSite] | None]:
    try:
        status, shown = ratchet_base.file_at_base(root, ALLOWLIST)
        if status != "present":
            return status, None
        assert shown is not None
        return "present", {
            LeakSite.parse(line.strip())
            for line in shown.splitlines()
            if line.strip() and not line.lstrip().startswith("#")
        }
    except Exception:
        return "unresolved", None


def repository_messages(root: Path) -> list[str]:
    entries, messages = load_manifest(root / MANIFEST)
    messages.extend(manifest_messages(root, entries))
    try:
        pr_phase_states = load_pr_phase_states(root)
    except ValueError as exc:
        messages.append(str(exc))
        pr_phase_states = set()
    leaks = current_leaks(root, entries, pr_phase_states)
    allowlist = load_allowlist(root / ALLOWLIST)
    base_status, base_allowlist = allowlist_at_dev_base(root)
    if base_status == "unresolved":
        messages.append("allowlist-base-unresolved: cannot resolve dev base allowlist to enforce shrink-only ratchet")
    messages.extend(ratchet_messages(leaks, allowlist, base_allowlist))
    return messages
