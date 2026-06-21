#!/usr/bin/env python3
"""Narrow regression guard for bare own-queue event.queue compares.

This DETECT-tier scan covers the idle_gate bug shape in department main.lua
files: a department declares a bare own queue in spec.consumes, then directly
compares event.queue or event["queue"] to that bare name in either operand
order. It is not a complete #551
namespaced-dispatch class harness. Regex cannot soundly cover alias-through-
local-variable flows, nested helpers, submodule guards, or arbitrary data flow.

The stronger follow-up is behavioral namespaced-dispatch conformance: for every
department, dispatch each spec.consumes queue under its production namespaced
name and assert it routes instead of falling to unknown-queue, unsupported, or
skip-foreign handling.
"""

from __future__ import annotations

import re
from pathlib import Path
from typing import Callable


LOCAL_SPEC_RE = re.compile(r"\blocal\s+spec\s*=\s*\{")
CONSUMES_RE = re.compile(r"\bconsumes\s*=\s*\{")
STRING_LITERAL_RE = re.compile(r"(?P<quote>[\"'])(?P<value>[^\"'\\]*(?:\\.[^\"'\\]*)*)(?P=quote)")
EVENT_QUEUE_COMPARE_RES = (
    re.compile(
        r"\bevent\s*(?:\.\s*queue|\[\s*(?P<field_quote>[\"'])queue(?P=field_quote)\s*\])"
        r"\s*(?P<op>==|~=)\s*(?P<queue_quote>[\"'])(?P<queue>[^\"']+)(?P=queue_quote)"
    ),
    re.compile(
        r"(?P<queue_quote>[\"'])(?P<queue>[^\"']+)(?P=queue_quote)\s*(?P<op>==|~=)\s*"
        r"\bevent\s*(?:\.\s*queue|\[\s*(?P<field_quote>[\"'])queue(?P=field_quote)\s*\])"
    ),
)


def line_number(source: str, index: int) -> int:
    return source.count("\n", 0, index) + 1


def matching_table_end(stripped: str, open_index: int) -> int | None:
    depth = 0
    for index in range(open_index, len(stripped)):
        char = stripped[index]
        if char == "{":
            depth += 1
        elif char == "}":
            depth -= 1
            if depth == 0:
                return index + 1
    return None


def table_span(pattern: re.Pattern[str], stripped: str, start: int = 0, end: int | None = None) -> tuple[int, int] | None:
    match = pattern.search(stripped, start, len(stripped) if end is None else end)
    if match is None:
        return None
    open_index = match.end() - 1
    close_index = matching_table_end(stripped, open_index)
    if close_index is None:
        return None
    if end is not None and close_index > end:
        return None
    return open_index, close_index


def code_string_literals(
    source: str,
    stripped: str,
    start: int,
    end: int,
    is_unmasked_range: Callable[[str, str, int, int], bool],
) -> list[str]:
    values: list[str] = []
    for match in STRING_LITERAL_RE.finditer(source, start, end):
        if not is_unmasked_range(source, stripped, match.start(), match.start("quote")):
            continue
        values.append(match.group("value"))
    return values


def bare_spec_consumes(
    source: str,
    strip_lua_comments_and_strings: Callable[[str], str],
    is_unmasked_range: Callable[[str, str, int, int], bool],
) -> set[str]:
    stripped = strip_lua_comments_and_strings(source)
    spec_span = table_span(LOCAL_SPEC_RE, stripped)
    if spec_span is None:
        return set()
    consumes_span = table_span(CONSUMES_RE, stripped, spec_span[0], spec_span[1])
    if consumes_span is None:
        return set()
    return {
        value
        for value in code_string_literals(source, stripped, consumes_span[0], consumes_span[1], is_unmasked_range)
        if "." not in value
    }


def compare_string_spans(match: re.Match[str]) -> list[tuple[int, int]]:
    spans = [(match.start("queue_quote"), match.end("queue") + 1)]
    field_quote_start = match.start("field_quote")
    if field_quote_start != -1:
        spans.append((field_quote_start, field_quote_start + len("'queue'")))
    return sorted(spans)


def compare_match_is_unmasked(
    source: str,
    stripped: str,
    match: re.Match[str],
    is_unmasked_range: Callable[[str, str, int, int], bool],
) -> bool:
    cursor = match.start()
    for start, end in compare_string_spans(match):
        if not is_unmasked_range(source, stripped, cursor, start):
            return False
        cursor = end
    return is_unmasked_range(source, stripped, cursor, match.end())


def bare_own_queue_compare_messages(
    path: str,
    source: str,
    strip_lua_comments_and_strings: Callable[[str], str],
    is_unmasked_range: Callable[[str, str, int, int], bool],
) -> list[str]:
    bare_consumes = bare_spec_consumes(source, strip_lua_comments_and_strings, is_unmasked_range)
    if not bare_consumes:
        return []

    stripped = strip_lua_comments_and_strings(source)
    messages: list[str] = []
    for pattern in EVENT_QUEUE_COMPARE_RES:
        for match in pattern.finditer(source):
            queue = match.group("queue")
            if "." in queue or queue not in bare_consumes:
                continue
            if not compare_match_is_unmasked(source, stripped, match, is_unmasked_range):
                continue
            messages.append(
                f"{path}:{line_number(source, match.start())} compares event.queue to bare own queue {queue!r}; "
                "production delivers namespaced queues, so use the router/saga contract instead"
            )
    return messages


def repository_messages(
    root: Path,
    packages_root: Path,
    read_text: Callable[[Path], str],
    rel: Callable[[Path, Path], str],
    strip_lua_comments_and_strings: Callable[[str], str],
    is_unmasked_range: Callable[[str, str, int, int], bool],
) -> list[str]:
    messages: list[str] = []
    if not packages_root.exists():
        return messages
    for path in sorted(packages_root.glob("*/departments/*/main.lua")):
        if not path.is_file():
            continue
        messages.extend(
            bare_own_queue_compare_messages(
                rel(root, path),
                read_text(path),
                strip_lua_comments_and_strings,
                is_unmasked_range,
            )
        )
    return messages
