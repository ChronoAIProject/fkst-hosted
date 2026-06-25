#!/usr/bin/env python3
"""Cross-package run_graph integration coverage shrink-only ratchet."""

from __future__ import annotations

import json
import re
from dataclasses import dataclass
from pathlib import Path


ALLOWLIST = "migration/integration-edge-coverage.allowlist"
LOCAL_SPEC_RE = re.compile(r"\blocal\s+spec\s*=\s*\{")
FIELD_RE_TEMPLATE = r"\b{field}\s*=\s*\{{"
STRING_RE = re.compile(r"(?P<quote>[\"'])(?P<value>[A-Za-z0-9_.-]+)(?P=quote)")
STRING_LITERAL_RE = re.compile(r"(?P<quote>[\"'])(?P<value>[^\"'\\]*(?:\\.[^\"'\\]*)*)(?P=quote)")
ASSERT_COVERS_RE = re.compile(r"\bgraph\s*\.\s*assert_covers\s*\(")


@dataclass(frozen=True, order=True)
class DepartmentSpec:
    package: str
    department: str
    path: str
    consumes: tuple[str, ...]
    produces: tuple[str, ...]

    def consumer_id(self) -> str:
        return f"{self.package}.{self.department}"


def mask_span(chars: list[str], start: int, end: int) -> None:
    for index in range(start, min(end, len(chars))):
        if chars[index] != "\n":
            chars[index] = " "


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


def strip_lua_comments_and_strings(text: str) -> str:
    chars = list(text)
    cursor = 0
    while cursor < len(text):
        if text.startswith("--", cursor):
            bracket = long_bracket_at(text, cursor + 2)
            if bracket is not None:
                opener_len, closer = bracket
                end = end_of_long_bracket(text, cursor + 2 + opener_len, closer)
            else:
                newline = text.find("\n", cursor)
                end = len(text) if newline == -1 else newline
            mask_span(chars, cursor, end)
            cursor = end
            continue
        char = text[cursor]
        if char in ("'", '"'):
            cursor = end_of_quoted_string(text, cursor)
            continue
        if char == "[":
            bracket = long_bracket_at(text, cursor)
            if bracket is not None:
                opener_len, closer = bracket
                cursor = end_of_long_bracket(text, cursor + opener_len, closer)
                continue
        cursor += 1
    return "".join(chars)


def matching_table_end(masked: str, open_index: int) -> int | None:
    depth = 0
    for index in range(open_index, len(masked)):
        char = masked[index]
        if char == "{":
            depth += 1
        elif char == "}":
            depth -= 1
            if depth == 0:
                return index + 1
    return None


def matching_call_end(masked: str, open_index: int) -> int:
    depth = 0
    for index in range(open_index, len(masked)):
        char = masked[index]
        if char == "(":
            depth += 1
        elif char == ")":
            depth -= 1
            if depth == 0:
                return index + 1
    return len(masked)


def table_span(pattern: re.Pattern[str], masked: str, start: int = 0, end: int | None = None) -> tuple[int, int] | None:
    match = pattern.search(masked, start, len(masked) if end is None else end)
    if match is None:
        return None
    open_index = match.end() - 1
    close_index = matching_table_end(masked, open_index)
    if close_index is None:
        return None
    if end is not None and close_index > end:
        return None
    return open_index, close_index


def code_string_literals(source: str, masked: str, start: int, end: int) -> list[str]:
    values: list[str] = []
    for match in STRING_RE.finditer(source, start, end):
        if masked[match.start("quote")] == source[match.start("quote")]:
            values.append(match.group("value"))
    return values


def spec_field_values(source: str, masked: str, spec_span: tuple[int, int], field: str) -> tuple[str, ...]:
    field_re = re.compile(FIELD_RE_TEMPLATE.format(field=re.escape(field)))
    span = table_span(field_re, masked, spec_span[0], spec_span[1])
    if span is None:
        return ()
    return tuple(dict.fromkeys(code_string_literals(source, masked, span[0], span[1])))


def package_dirs(root: Path) -> list[Path]:
    packages = root / "packages"
    if not packages.exists():
        return []
    return [path for path in sorted(packages.iterdir()) if path.is_dir()]


def package_for_queue(current_package: str, queue: str) -> str:
    return queue.split(".", 1)[0] if "." in queue else current_package


def normalize_queue(current_package: str, queue: str) -> str:
    return queue if "." in queue else f"{current_package}.{queue}"


def department_specs(root: Path) -> set[DepartmentSpec]:
    specs: set[DepartmentSpec] = set()
    for package in package_dirs(root):
        for path in sorted(package.glob("departments/*/main.lua")):
            source = path.read_text(encoding="utf-8")
            masked = strip_lua_comments_and_strings(source)
            spec_span = table_span(LOCAL_SPEC_RE, masked)
            if spec_span is None:
                continue
            specs.add(
                DepartmentSpec(
                    package=package.name,
                    department=path.parent.name,
                    path="packages/" + path.relative_to(root / "packages").as_posix(),
                    consumes=spec_field_values(source, masked, spec_span, "consumes"),
                    produces=spec_field_values(source, masked, spec_span, "produces"),
                )
            )
    return specs


def cross_package_edges(root: Path) -> set[str]:
    specs = department_specs(root)
    producers_by_queue: dict[str, set[str]] = {}
    for spec in specs:
        for queue in spec.produces:
            producers_by_queue.setdefault(normalize_queue(spec.package, queue), set()).add(spec.package)

    edges: set[str] = set()
    for spec in specs:
        for queue in spec.consumes:
            normalized = normalize_queue(spec.package, queue)
            consumer_package = spec.package
            for producer_package in producers_by_queue.get(normalized, set()):
                if producer_package != consumer_package:
                    edges.add(f"{normalized} -> {spec.consumer_id()}")
    return edges


def run_graph_test_files(root: Path) -> list[Path]:
    packages = root / "packages"
    if not packages.exists():
        return []
    return sorted(packages.glob("*/tests/run_graph*.lua"))


def observed_edges(root: Path) -> set[str]:
    edges: set[str] = set()
    for path in run_graph_test_files(root):
        source = path.read_text(encoding="utf-8")
        masked = strip_lua_comments_and_strings(source)
        for match in ASSERT_COVERS_RE.finditer(masked):
            call_end = matching_call_end(masked, match.end() - 1)
            for string_match in STRING_LITERAL_RE.finditer(source, match.end(), call_end):
                edge = string_match.group("value")
                if " -> " in edge:
                    edges.add(edge)
    return edges


def load_allowlist(path: Path) -> set[str]:
    if not path.exists():
        return set()
    entries: set[str] = set()
    for number, raw in enumerate(path.read_text(encoding="utf-8").splitlines(), start=1):
        line = raw.strip()
        if not line or line.startswith("#"):
            continue
        try:
            item = json.loads(line)
        except json.JSONDecodeError as exc:
            raise ValueError(f"invalid {ALLOWLIST} JSON on line {number}: {exc.msg}") from exc
        if not isinstance(item, dict):
            raise ValueError(f"invalid {ALLOWLIST} line {number}: expected JSON object")
        edge = item.get("edge")
        reason = item.get("reason")
        if not isinstance(edge, str) or " -> " not in edge:
            raise ValueError(f"invalid {ALLOWLIST} line {number}: edge is required")
        if not isinstance(reason, str) or reason.strip() == "":
            raise ValueError(f"invalid {ALLOWLIST} line {number}: reason is required")
        entries.add(edge)
    return entries


def ratchet_messages(edges: set[str], observed: set[str], allowlist: set[str]) -> list[str]:
    messages: list[str] = []
    uncovered = edges - observed
    for edge in sorted(uncovered - allowlist):
        messages.append(
            f"new uncovered cross-package edge {edge}: add a run_graph test covering it (graph.assert_covers), shrink-only ratchet"
        )
    for edge in sorted(allowlist & observed):
        messages.append(f"stale: remove {edge}, now covered")
    for edge in sorted(allowlist - edges):
        messages.append(f"stale: {edge} no longer exists")
    return messages


def repository_messages(root: Path) -> list[str]:
    return ratchet_messages(
        cross_package_edges(root),
        observed_edges(root),
        load_allowlist(root / ALLOWLIST),
    )


def main() -> int:
    messages = repository_messages(Path.cwd())
    if messages:
        print("integration coverage check failed:")
        for message in messages:
            print(f"  {message}")
        return 1
    print("OK: integration coverage ratchet passed")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
