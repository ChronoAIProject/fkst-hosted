#!/usr/bin/env python3
"""Surface ratchet for github-devloop-intake-default capability cleanup."""

from __future__ import annotations

import re
from dataclasses import dataclass
from pathlib import Path


INTAKE_DEFAULT_PACKAGE = "github-devloop-intake-default"
CANONICAL_RISK_PATH = "libraries/devloop/github_risk.lua"
DELETED_CAPABILITIES_REQUIRE = "core.github_capabilities"
HIGH_RISK_DEF_RE = re.compile(
    r"\b(?:function\s+[A-Za-z_][A-Za-z0-9_]*\s*\.\s*(?P<function>github_high_risk_paths?)\s*\("
    r"|[A-Za-z_][A-Za-z0-9_]*\s*\.\s*(?P<assign>github_high_risk_paths?)\s*=\s*function\b)"
)
EXPORT_ASSIGN_RE = re.compile(
    r"\b(?:function\s+M\s*\.\s*(?P<function>[A-Za-z_][A-Za-z0-9_]*)\s*\("
    r"|M\s*\.\s*(?P<assign>[A-Za-z_][A-Za-z0-9_]*)\s*=)"
)
REQUIRE_CAPABILITIES_RE = re.compile(
    r"\brequire\s*(?:\(\s*)?(?P<quote>[\"'])core\.github_capabilities(?P=quote)"
)


@dataclass(frozen=True)
class Source:
    relpath: str
    path: Path
    text: str

    @property
    def package(self) -> str | None:
        parts = Path(self.relpath).parts
        if len(parts) >= 2 and parts[0] == "packages":
            return parts[1]
        return None


def _mask(chars: list[str], start: int, end: int) -> None:
    for index in range(start, end):
        if chars[index] != "\n":
            chars[index] = " "


def quoted_string_end(text: str, start: int) -> int:
    quote = text[start]
    index = start + 1
    while index < len(text):
        if text[index] == "\\":
            index += 2
            continue
        if text[index] == quote:
            return index + 1
        index += 1
    return len(text)


def long_bracket_at(text: str, index: int) -> tuple[int, str] | None:
    if index >= len(text) or text[index] != "[":
        return None
    cursor = index + 1
    while cursor < len(text) and text[cursor] == "=":
        cursor += 1
    if cursor >= len(text) or text[cursor] != "[":
        return None
    level = cursor - index - 1
    return cursor - index + 1, "]" + ("=" * level) + "]"


def long_bracket_end(text: str, body_start: int, closer: str) -> int:
    close_start = text.find(closer, body_start)
    return len(text) if close_start == -1 else close_start + len(closer)


def lua_code_mask(text: str) -> str:
    chars = list(text)
    index = 0
    while index < len(text):
        if text.startswith("--", index):
            long = long_bracket_at(text, index + 2)
            if long is not None:
                opener_len, closer = long
                end = long_bracket_end(text, index + 2 + opener_len, closer)
                _mask(chars, index, end)
                index = end
                continue
            newline = text.find("\n", index)
            end = len(text) if newline == -1 else newline
            _mask(chars, index, end)
            index = end
            continue
        long = long_bracket_at(text, index)
        if long is not None:
            opener_len, closer = long
            end = long_bracket_end(text, index + opener_len, closer)
            _mask(chars, index, end)
            index = end
            continue
        char = text[index]
        if char in {"'", '"'}:
            end = quoted_string_end(text, index)
            _mask(chars, index, end)
            index = end
            continue
        index += 1
    return "".join(chars)


def line_number(text: str, index: int) -> int:
    return text.count("\n", 0, index) + 1


def lua_sources(root: Path) -> list[Source]:
    result: list[Source] = []
    for base in (root / "packages", root / "libraries"):
        if not base.exists():
            continue
        for path in sorted(base.rglob("*.lua")):
            if path.is_file():
                result.append(Source(path.relative_to(root).as_posix(), path, path.read_text(encoding="utf-8")))
    return result


def is_test_source(source: Source) -> bool:
    return "tests" in Path(source.relpath).parts


def high_risk_definitions(sources: list[Source], name: str) -> list[tuple[Source, int]]:
    found: list[tuple[Source, int]] = []
    for source in sources:
        if is_test_source(source):
            continue
        masked = lua_code_mask(source.text)
        for match in HIGH_RISK_DEF_RE.finditer(masked):
            if (match.group("function") or match.group("assign")) == name:
                found.append((source, match.start()))
    return found


def high_risk_messages(sources: list[Source]) -> list[str]:
    messages: list[str] = []
    for name in ("github_high_risk_path", "github_high_risk_paths"):
        found = high_risk_definitions(sources, name)
        canonical = [source.relpath for source, _ in found if source.relpath == CANONICAL_RISK_PATH]
        if len(found) == 1 and len(canonical) == 1:
            continue
        locations = ", ".join(
            f"{source.relpath}:{line_number(source.text, index)}" for source, index in found
        ) or "none"
        messages.append(
            f"expected exactly one typed {name} definition in {CANONICAL_RISK_PATH}; found {locations}"
        )
    return messages


def forbidden_export_kind(name: str) -> str | None:
    if name == "github_command_capability" or name.startswith("github_capability_"):
        return "GitHub capability"
    if name.startswith("github_prompt_injection_") and "canary" in name:
        return "GitHub prompt-injection canary"
    return None


def intake_default_surface_messages(sources: list[Source]) -> list[str]:
    messages: list[str] = []
    for source in sources:
        if source.package is None or is_test_source(source):
            continue
        masked = lua_code_mask(source.text)
        if source.relpath == f"packages/{INTAKE_DEFAULT_PACKAGE}/core.lua":
            for match in REQUIRE_CAPABILITIES_RE.finditer(source.text):
                if masked[match.start():match.start("quote")].strip() == "":
                    continue
                messages.append(
                    f"{source.relpath}:{line_number(source.text, match.start())} must not require {DELETED_CAPABILITIES_REQUIRE}"
                )
        for match in EXPORT_ASSIGN_RE.finditer(masked):
            name = match.group("function") or match.group("assign") or ""
            kind = forbidden_export_kind(name)
            if kind is None:
                continue
            messages.append(
                f"{source.relpath}:{line_number(source.text, match.start())} package-private {kind} export is forbidden: {name}"
            )
    return messages


def repository_messages(root: Path) -> list[str]:
    sources = lua_sources(root)
    return high_risk_messages(sources) + intake_default_surface_messages(sources)
