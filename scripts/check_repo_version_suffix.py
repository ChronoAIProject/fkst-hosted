#!/usr/bin/env python3
"""Shrink-only ratchet for raw transition-version suffix handling."""

from __future__ import annotations

import re
from dataclasses import dataclass
from pathlib import Path

import ratchet_base


ALLOWLIST = "migration/version-suffix.allowlist"
SANCTIONED_PATH = "libraries/contract/transition_version.lua"
SUFFIX_LITERALS = (
    "/loop/",
    "/fix/",
    "/review-loop/",
    "/review-meta-action/",
    "/ready-split/",
    "/reimplement/",
    "/timeout/",
)
PARSING_FUNCTION_RE = re.compile(r"(?P<call>[:.]\s*(?:gmatch|match|gsub)|\bstring\s*\.\s*(?:gmatch|match|gsub))\s*\(")
PARSING_PATTERN_RE = re.compile(
    r"/(?:loop|fix|review%-loop|review%-meta%-action|ready%-split|reimplement|timeout)/"
    r"|\[/-\](?:loop|fix|review%-loop|review%-meta%-action|ready%-split|reimplement|timeout)"
    r"|/(?:review%-loop|review%-meta%-action|ready%-split)/"
)


@dataclass(frozen=True, order=True)
class LuaStringLiteral:
    start: int
    end: int
    content: str


@dataclass(frozen=True, order=True)
class VersionSuffixSite:
    path: str
    line: int
    kind: str
    text: str

    def key(self) -> tuple[str, int]:
        return self.path, self.line

    def label(self) -> str:
        return f"{self.path}:{self.line} {self.kind} {self.text}"


@dataclass(frozen=True, order=True)
class VersionSuffixAllowlistEntry:
    path: str
    line: int
    why: str

    @classmethod
    def parse(cls, line: str) -> "VersionSuffixAllowlistEntry":
        match = re.fullmatch(r"(?P<path>[^:\s]+):(?P<line>\d+)\s+#\s+why=(?P<why>.+)", line)
        if match is None:
            raise ValueError(f"invalid {ALLOWLIST} line: {line}")
        path = match.group("path")
        if not (path.startswith("libraries/") or path.startswith("packages/")) or not path.endswith(".lua"):
            raise ValueError(f"invalid {ALLOWLIST} path: {line}")
        why = match.group("why").strip()
        if not why:
            raise ValueError(f"invalid {ALLOWLIST} WHY: {line}")
        return cls(path=path, line=int(match.group("line")), why=why)

    def key(self) -> tuple[str, int]:
        return self.path, self.line

    def label(self) -> str:
        return f"{self.path}:{self.line}"


def long_bracket_at(text: str, index: int) -> tuple[int, str] | None:
    if index >= len(text) or text[index] != "[":
        return None
    cursor = index + 1
    while cursor < len(text) and text[cursor] == "=":
        cursor += 1
    if cursor >= len(text) or text[cursor] != "[":
        return None
    return cursor - index + 1, "]" + ("=" * (cursor - index - 1)) + "]"


def end_of_long_bracket(text: str, body_start: int, closer: str) -> int:
    close_start = text.find(closer, body_start)
    return len(text) if close_start == -1 else close_start + len(closer)


def end_of_quoted_string(text: str, start: int) -> int:
    quote = text[start]
    cursor = start + 1
    while cursor < len(text):
        if text[cursor] == "\\":
            cursor += 2
            continue
        if text[cursor] == quote:
            return cursor + 1
        cursor += 1
    return len(text)


def mask_span(chars: list[str], start: int, end: int) -> None:
    for index in range(start, min(end, len(chars))):
        if chars[index] != "\n":
            chars[index] = " "


def lua_code_mask(text: str) -> str:
    chars = list(text)
    cursor = 0
    while cursor < len(text):
        if text.startswith("--", cursor):
            bracket = long_bracket_at(text, cursor + 2)
            if bracket is None:
                newline = text.find("\n", cursor)
                end = len(text) if newline == -1 else newline
            else:
                opener_len, closer = bracket
                end = end_of_long_bracket(text, cursor + 2 + opener_len, closer)
            mask_span(chars, cursor, end)
            cursor = end
            continue
        if text[cursor] in ("'", '"'):
            end = end_of_quoted_string(text, cursor)
            mask_span(chars, cursor, end)
            cursor = end
            continue
        bracket = long_bracket_at(text, cursor)
        if bracket is not None:
            opener_len, closer = bracket
            end = end_of_long_bracket(text, cursor + opener_len, closer)
            mask_span(chars, cursor, end)
            cursor = end
            continue
        cursor += 1
    return "".join(chars)


def lua_string_literals(text: str) -> list[LuaStringLiteral]:
    literals: list[LuaStringLiteral] = []
    cursor = 0
    while cursor < len(text):
        if text.startswith("--", cursor):
            bracket = long_bracket_at(text, cursor + 2)
            if bracket is None:
                newline = text.find("\n", cursor)
                cursor = len(text) if newline == -1 else newline
            else:
                opener_len, closer = bracket
                cursor = end_of_long_bracket(text, cursor + 2 + opener_len, closer)
            continue
        if text[cursor] in ("'", '"'):
            start = cursor
            end = end_of_quoted_string(text, cursor)
            content_end = end - 1 if end <= len(text) and text[end - 1] == text[cursor] else end
            literals.append(LuaStringLiteral(start=start, end=end, content=text[cursor + 1 : content_end]))
            cursor = end
            continue
        bracket = long_bracket_at(text, cursor)
        if bracket is not None:
            start = cursor
            opener_len, closer = bracket
            body_start = cursor + opener_len
            close_start = text.find(closer, body_start)
            body_end = len(text) if close_start == -1 else close_start
            end = len(text) if close_start == -1 else close_start + len(closer)
            literals.append(LuaStringLiteral(start=start, end=end, content=text[body_start:body_end]))
            cursor = end
            continue
        cursor += 1
    return literals


def line_start(text: str, index: int) -> int:
    newline = text.rfind("\n", 0, index)
    return 0 if newline == -1 else newline + 1


def line_end(text: str, index: int) -> int:
    newline = text.find("\n", index)
    return len(text) if newline == -1 else newline


def line_number(text: str, index: int) -> int:
    return text.count("\n", 0, index) + 1


def line_text(text: str, index: int) -> str:
    return text[line_start(text, index) : line_end(text, index)].strip()


def has_concat_neighbor(masked: str, literal: LuaStringLiteral) -> bool:
    before = masked[line_start(masked, literal.start) : literal.start]
    after = masked[literal.end : line_end(masked, literal.end)]
    return ".." in before or ".." in after


def contains_banned_suffix_literal(content: str) -> bool:
    return any(suffix in content for suffix in SUFFIX_LITERALS)


def parsing_call_span(masked: str, literal: LuaStringLiteral) -> tuple[int, int] | None:
    start = max(0, literal.start - 160)
    prefix = masked[start : literal.start]
    match = None
    for candidate in PARSING_FUNCTION_RE.finditer(prefix):
        match = candidate
    if match is None:
        return None
    open_pos = start + match.end() - 1
    return open_pos, literal.start


def matching_paren_end(masked: str, open_pos: int) -> int:
    depth = 0
    cursor = open_pos
    while cursor < len(masked):
        char = masked[cursor]
        if char == "(":
            depth += 1
        elif char == ")":
            depth -= 1
            if depth == 0:
                return cursor + 1
        cursor += 1
    return len(masked)


def is_parsing_pattern(masked: str, literal: LuaStringLiteral) -> bool:
    if PARSING_PATTERN_RE.search(literal.content) is None:
        return False
    span = parsing_call_span(masked, literal)
    if span is None or "\n" in masked[span[0] : span[1]]:
        return False
    return literal.start < matching_paren_end(masked, span[0])


def source_sites(path: str, source: str) -> set[VersionSuffixSite]:
    masked = lua_code_mask(source)
    sites: set[VersionSuffixSite] = set()
    for literal in lua_string_literals(source):
        if contains_banned_suffix_literal(literal.content) and has_concat_neighbor(masked, literal):
            sites.add(VersionSuffixSite(path, line_number(source, literal.start), "construction", line_text(source, literal.start)))
        if is_parsing_pattern(masked, literal):
            sites.add(VersionSuffixSite(path, line_number(source, literal.start), "parsing", line_text(source, literal.start)))
    return sites


def is_production_lua_path(root: Path, path: Path) -> bool:
    if path.suffix != ".lua":
        return False
    relpath = path.relative_to(root).as_posix()
    if relpath == SANCTIONED_PATH:
        return False
    if path.name.endswith("_test.lua"):
        return False
    if "tests" in path.relative_to(root).parts:
        return False
    return relpath.startswith(("libraries/", "packages/"))


def production_lua_sources(root: Path) -> dict[str, str]:
    sources: dict[str, str] = {}
    for dirname in ("libraries", "packages"):
        scan_root = root / dirname
        if not scan_root.exists():
            continue
        for path in sorted(scan_root.rglob("*.lua")):
            if path.is_file() and is_production_lua_path(root, path):
                sources[path.relative_to(root).as_posix()] = path.read_text(encoding="utf-8")
    return sources


def sites(root: Path) -> set[VersionSuffixSite]:
    current: set[VersionSuffixSite] = set()
    for path, source in production_lua_sources(root).items():
        current.update(source_sites(path, source))
    return current


def load_allowlist(path: Path) -> set[VersionSuffixAllowlistEntry]:
    if not path.exists():
        return set()
    entries: set[VersionSuffixAllowlistEntry] = set()
    for raw in path.read_text(encoding="utf-8").splitlines():
        stripped = raw.strip()
        if not stripped or stripped.startswith("#"):
            continue
        entries.add(VersionSuffixAllowlistEntry.parse(stripped))
    return entries


def ratchet_messages(
    current: set[VersionSuffixSite],
    allowlist: set[VersionSuffixAllowlistEntry],
    base_allowlist: set[VersionSuffixAllowlistEntry] | None = None,
) -> list[str]:
    messages: list[str] = []
    allow_keys = {entry.key() for entry in allowlist}
    current_keys = {site.key() for site in current}
    for site in sorted(current):
        if site.key() not in allow_keys:
            messages.append(f"{site.label()} constructs/parses a transition-version suffix outside contract.transition_version")
    for entry in sorted(allowlist):
        if entry.key() not in current_keys:
            messages.append(f"{entry.label()} no longer matches transition-version suffix debt; prune the stale entry")
    if base_allowlist is not None:
        base_keys = {entry.key() for entry in base_allowlist}
        for entry in sorted(allowlist):
            if entry.key() not in base_keys:
                messages.append(f"{entry.label()} grows version-suffix allowlist relative to dev; migrate to contract.transition_version instead")
    return messages


def allowlist_at_dev_base(root: Path) -> tuple[str, set[VersionSuffixAllowlistEntry] | None]:
    try:
        status, shown = ratchet_base.file_at_base(root, ALLOWLIST)
        if status != "present":
            return status, None
        assert shown is not None
        return "present", {
            VersionSuffixAllowlistEntry.parse(line.strip())
            for line in shown.splitlines()
            if line.strip() and not line.lstrip().startswith("#")
        }
    except Exception:
        return "unresolved", None


def repository_messages(
    root: Path,
    allowlist_dir: Path | None = None,
    enforce_base: bool = True,
) -> list[str]:
    current = sites(root)
    allow_path = root / ALLOWLIST if allowlist_dir is None else allowlist_dir / Path(ALLOWLIST).name
    allowlist = load_allowlist(allow_path)
    base_status, base_allowlist = allowlist_at_dev_base(root) if enforce_base else ("absent", None)
    messages: list[str] = []
    if base_status == "unresolved":
        messages.append("cannot resolve dev base allowlist to enforce shrink-only ratchet; ensure CI provides the dev ref")
    messages.extend(ratchet_messages(current, allowlist, base_allowlist))
    return messages


if __name__ == "__main__":
    found = repository_messages(Path.cwd(), enforce_base=False)
    for message in found:
        print(message)
    raise SystemExit(1 if found else 0)
