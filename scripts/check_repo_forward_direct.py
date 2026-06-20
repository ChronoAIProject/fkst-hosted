#!/usr/bin/env python3
"""Forward-direct marker-gated raise ratchet for github-devloop."""

from __future__ import annotations

import re
from dataclasses import dataclass
from pathlib import Path

ALLOWLIST = "migration/forward-direct-raise.allowlist"
MARKER_GATED_QUEUES = {
    "devloop_ready",
    "devloop_reviewing",
    "devloop_fixing",
    "devloop_merge_ready",
    "devloop_open_pr",
    "devloop_reconcile",
}
REDRIVE_PATHS = {
    "packages/github-devloop/core/pr_review_replayer.lua",
    "packages/github-devloop/core/ready_split.lua",
    "packages/github-devloop/core/replayer.lua",
    "packages/github-devloop/departments/observe_issue/main.lua",
}
CAUSAL_PATHS = {
    "packages/github-devloop/departments/comment_handoff/main.lua",
}
LOG_RAISE_RE = re.compile(r"\b(?:core|M)\s*\.\s*log_raise\s*\((?P<args>[^\n]*)")
RAW_RAISE_RE = re.compile(r"\braise\s*\(\s*(?P<quote>[\"'])(?P<queue>devloop_[A-Za-z0-9_]+)(?P=quote)")
QUEUE_EFFECT_RE = re.compile(r"\bqueue\s*=\s*(?P<quote>[\"'])(?P<queue>devloop_[A-Za-z0-9_]+)(?P=quote)")
LITERAL_ARG_RE = re.compile(r"(?P<quote>[\"'])(?P<value>devloop_[A-Za-z0-9_]+)(?P=quote)")
DYNAMIC_QUEUE_HINTS = {
    "recovery.queue": "devloop_fixing",
}
FUNCTION_RE = re.compile(
    r"^\s*(?:local\s+)?function\s+(?P<name>[A-Za-z_][A-Za-z0-9_]*(?:[.:][A-Za-z_][A-Za-z0-9_]*)?)\b"
)
SAGA_ACT_FUNCTION_RE = re.compile(r"\bact\s*=\s*function\s*\(")


@dataclass(frozen=True, order=True)
class ForwardDirectSite:
    path: str
    function: str
    queue: str

    def allowlist_line(self) -> str:
        return f"{self.path}|{self.function}|{self.queue}"

    def label(self) -> str:
        return f"{self.path}::{self.function} -> {self.queue}"


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
        if char in ("'", '"'):
            quote = char
            index += 1
            continue
        if line.startswith("--", index):
            return line[:index]
        index += 1
    return line


def _paren_delta(line: str) -> int:
    return line.count("(") - line.count(")")


def _literal_raise_queues(line: str) -> set[str]:
    queues: set[str] = set()
    for match in LOG_RAISE_RE.finditer(line):
        for literal in LITERAL_ARG_RE.finditer(match.group("args")):
            queues.add(literal.group("value"))
    for match in RAW_RAISE_RE.finditer(line):
        queues.add(match.group("queue"))
    for expression, queue in DYNAMIC_QUEUE_HINTS.items():
        if f", {expression}," in line:
            queues.add(queue)
    return queues & MARKER_GATED_QUEUES


def source_sites(path: str, text: str) -> set[ForwardDirectSite]:
    if path in REDRIVE_PATHS or path in CAUSAL_PATHS:
        return set()
    sites: set[ForwardDirectSite] = set()
    function = "chunk"
    in_raise_effects = False
    raise_effects_depth = 0
    for raw in text.splitlines():
        line = _strip_lua_line_comment(raw)
        match = FUNCTION_RE.match(line)
        if match is not None:
            function = match.group("name")
        elif SAGA_ACT_FUNCTION_RE.search(line):
            function = "pipeline"
        queues = _literal_raise_queues(line)
        if "raise_effects" in line and "(" in line:
            in_raise_effects = True
            raise_effects_depth = max(1, _paren_delta(line))
        if in_raise_effects:
            for effect in QUEUE_EFFECT_RE.finditer(line):
                queues.add(effect.group("queue"))
            raise_effects_depth += _paren_delta(line) if "raise_effects" not in line else 0
            if raise_effects_depth <= 0:
                in_raise_effects = False
        for queue in queues & MARKER_GATED_QUEUES:
            sites.add(ForwardDirectSite(path, function, queue))
    return sites


def sources(root: Path) -> dict[str, str]:
    base = root / "packages" / "github-devloop"
    if not base.exists():
        return {}
    found: dict[str, str] = {}
    for path in sorted(base.rglob("*.lua")):
        if "/tests/" in path.as_posix():
            continue
        rel = path.relative_to(root).as_posix()
        found[rel] = path.read_text(encoding="utf-8")
    return found


def current_sites(source_map: dict[str, str]) -> set[ForwardDirectSite]:
    sites: set[ForwardDirectSite] = set()
    for path, text in source_map.items():
        sites.update(source_sites(path, text))
    return sites


def load_allowlist(path: Path) -> set[ForwardDirectSite]:
    if not path.exists():
        return set()
    allowed: set[ForwardDirectSite] = set()
    for raw in path.read_text(encoding="utf-8").splitlines():
        line = raw.strip()
        if line == "" or line.startswith("#"):
            continue
        parts = line.split("|")
        if len(parts) != 3 or any(part == "" for part in parts):
            raise ValueError(f"invalid forward-direct allowlist line: {raw}")
        allowed.add(ForwardDirectSite(parts[0], parts[1], parts[2]))
    return allowed


def ratchet_messages(current: set[ForwardDirectSite], allowlist: set[ForwardDirectSite]) -> list[str]:
    messages: list[str] = []
    for site in sorted(current - allowlist):
        messages.append(f"{site.label()} is a new marker-gated FORWARD-direct raise not in {ALLOWLIST}; route it through github_comment_written -> comment_handoff or declare REDRIVE")
    for site in sorted(allowlist - current):
        messages.append(f"{site.label()} is listed in {ALLOWLIST} but no longer exists; remove the stale allowlist entry")
    return messages


def repository_messages(root: Path) -> list[str]:
    return ratchet_messages(current_sites(sources(root)), load_allowlist(root / ALLOWLIST))
