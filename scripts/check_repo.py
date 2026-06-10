#!/usr/bin/env python3
"""Hermetic repository guards for fkst packages.

This file is a fast, best-effort static lint. It checks repository-local
invariants that do not require the engine binary: source file size, test file
naming, helper shape, helper reachability, and whether a test file appears to
contain a test definition.

It is not a Lua parser and does not decide which top-level returned-table
``test_`` keys the engine actually enumerates. The authoritative coverage check
is the full-suite engine report-json audit in scripts/run.sh. A deeper
engine-loader-based Lua audit remains an engine-PR backlog item.
"""

from __future__ import annotations

import re
import sys
from dataclasses import dataclass
from pathlib import Path


LINE_LIMIT = 1000
SOURCE_SUFFIXES = {".lua", ".sh", ".py", ".rs"}
TEST_DEF_RE = re.compile(
    r"\b(?P<bare>test_[A-Za-z0-9_]+)\s*=\s*function\b"
    r"|\[\s*(?P<key_quote>[\"'])(?P<bracket>test_[A-Za-z0-9_]+)(?P=key_quote)\s*\]\s*=\s*function\b"
)
TEST_ASSIGN_RE = re.compile(
    r"\b(?P<bare>test_[A-Za-z0-9_]+)\s*="
    r"|\[\s*(?P<assign_key_quote>[\"'])(?P<bracket>test_[A-Za-z0-9_]+)(?P=assign_key_quote)\s*\]\s*="
    r"|\b[A-Za-z_][A-Za-z0-9_]*\s*\.\s*(?P<field>test_[A-Za-z0-9_]+)\s*="
    r"|\b[A-Za-z_][A-Za-z0-9_]*\s*\[\s*(?P<field_key_quote>[\"'])(?P<field_bracket>test_[A-Za-z0-9_]+)(?P=field_key_quote)\s*\]\s*="
)
TEST_FUNCTION_SUGAR_RE = re.compile(
    r"\bfunction\s+(?P<bare>test_[A-Za-z0-9_]+)\s*\("
    r"|\bfunction\s+[A-Za-z_][A-Za-z0-9_]*\s*[.:]\s*(?P<field>test_[A-Za-z0-9_]+)\s*\("
)
TEST_NAME_RE = re.compile(r"test_[A-Za-z0-9_]+\Z")
TEST_REQUIRE_RE = re.compile(
    r"\brequire\s*(?P<open_parens>(?:\(\s*)*)"
    r"(?:(?P<quote>[\"'])tests\.(?P<quoted>[A-Za-z0-9_.-]+)(?P=quote)"
    r"|(?P<long_literal>\[(?P<long_eq>=*)\[tests\.(?P<long>[A-Za-z0-9_.-]+)\](?P=long_eq)\]))"
    r"\s*(?P<close_parens>\)*)"
)
GRAPHQL_FIRST_CONNECTION_RE = re.compile(
    r"\b[A-Za-z_][A-Za-z0-9_]*\s*"
    r"\([^(){}]*\bfirst\s*:\s*\d+\b[^(){}]*\)\s*\{",
    re.DOTALL,
)
LONG_STRING_CHAR_RE = re.compile(r"\bstring\s*\.\s*char\s*\((?P<args>[^)]*)\)", re.DOTALL)
NUMERIC_ARG_RE = re.compile(r"(?:^|,)\s*\d+\s*(?=,|\Z)")
HIDDEN_TEXT_STRING_CHAR_ARG_MIN = 6
ERROR_CALL_STRING_RE = re.compile(r"\berror\s*\(\s*(?P<quote>['\"])(?P<message>[^'\"]*)(?P=quote)")
ERROR_CLASS_PREFIX_RE = re.compile(r"^[a-z0-9][a-z0-9-]*: [a-z0-9][a-z0-9-]*:")


@dataclass(frozen=True)
class LuaStringLiteral:
    line: int
    content: str


def repo_root() -> Path:
    return Path(__file__).resolve().parents[1]


def rel(root: Path, path: Path) -> str:
    return path.relative_to(root).as_posix()


def read_text(path: Path) -> str:
    return path.read_text(encoding="utf-8")


def line_count(path: Path) -> int:
    return len(read_text(path).splitlines())


def add(violations: list[str], rule: str, message: str) -> None:
    violations.append(f"{rule}: {message}")


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


def mask_span(chars: list[str], start: int, end: int) -> None:
    for index in range(start, end):
        if chars[index] != "\n":
            chars[index] = " "


def end_of_long_bracket(text: str, body_start: int, closer: str) -> int:
    close_start = text.find(closer, body_start)
    if close_start == -1:
        return len(text)
    return close_start + len(closer)


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


def bracket_test_assignment_key_string_end(text: str, quote_start: int) -> int | None:
    quote = text[quote_start]
    string_end = end_of_quoted_string(text, quote_start)
    if string_end > len(text) or text[string_end - 1] != quote:
        return None
    if not TEST_NAME_RE.fullmatch(text[quote_start + 1 : string_end - 1]):
        return None

    cursor = quote_start - 1
    while cursor >= 0 and text[cursor].isspace():
        cursor -= 1
    if cursor < 0 or text[cursor] != "[":
        return None

    cursor = string_end
    while cursor < len(text) and text[cursor].isspace():
        cursor += 1
    if cursor >= len(text) or text[cursor] != "]":
        return None
    cursor += 1
    while cursor < len(text) and text[cursor].isspace():
        cursor += 1
    if cursor >= len(text) or text[cursor] != "=":
        return None
    return string_end


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
            end = end_of_quoted_string(text, cursor)
            if bracket_test_assignment_key_string_end(text, cursor) is None:
                mask_span(chars, cursor, end)
            cursor = end
            continue

        if char == "[":
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
            if bracket is not None:
                opener_len, closer = bracket
                cursor = end_of_long_bracket(text, cursor + 2 + opener_len, closer)
            else:
                newline = text.find("\n", cursor)
                cursor = len(text) if newline == -1 else newline
            continue

        char = text[cursor]
        if char in ("'", '"'):
            end = end_of_quoted_string(text, cursor)
            content_end = end - 1 if end <= len(text) and text[end - 1] == char else end
            literals.append(
                LuaStringLiteral(
                    line=text.count("\n", 0, cursor) + 1,
                    content=text[cursor + 1 : content_end],
                )
            )
            cursor = end
            continue

        if char == "[":
            bracket = long_bracket_at(text, cursor)
            if bracket is not None:
                opener_len, closer = bracket
                body_start = cursor + opener_len
                close_start = text.find(closer, body_start)
                body_end = len(text) if close_start == -1 else close_start
                literals.append(
                    LuaStringLiteral(
                        line=text.count("\n", 0, cursor) + 1,
                        content=text[body_start:body_end],
                    )
                )
                cursor = len(text) if close_start == -1 else close_start + len(closer)
                continue

        cursor += 1
    return literals


def matching_graphql_brace(text: str, open_index: int) -> int | None:
    depth = 0
    for index in range(open_index, len(text)):
        char = text[index]
        if char == "{":
            depth += 1
        elif char == "}":
            depth -= 1
            if depth == 0:
                return index
    return None


def graphql_top_level_text(text: str) -> str:
    chars: list[str] = []
    depth = 0
    for char in text:
        if char == "{":
            depth += 1
            chars.append(" ")
        elif char == "}":
            depth = max(0, depth - 1)
            chars.append(" ")
        elif depth == 0:
            chars.append(char)
        elif char == "\n":
            chars.append("\n")
        else:
            chars.append(" ")
    return "".join(chars)


def graphql_depth_at(text: str, index: int) -> int:
    depth = 0
    for char in text[:index]:
        if char == "{":
            depth += 1
        elif char == "}":
            depth = max(0, depth - 1)
    return depth


def graphql_top_level_field_body(text: str, field_name: str) -> str | None:
    field_re = re.compile(r"\b" + re.escape(field_name) + r"\b\s*\{")
    for match in field_re.finditer(text):
        if graphql_depth_at(text, match.start()) != 0:
            continue
        open_index = match.end() - 1
        close_index = matching_graphql_brace(text, open_index)
        if close_index is not None:
            return text[open_index + 1 : close_index]
    return None


def graphql_connection_has_truncation_guard(selection_body: str) -> bool:
    top_level = graphql_top_level_text(selection_body)
    if re.search(r"\btotalCount\b", top_level):
        return True

    page_info_body = graphql_top_level_field_body(selection_body, "pageInfo")
    if page_info_body is None:
        return False
    return re.search(r"\bhasNextPage\b", graphql_top_level_text(page_info_body)) is not None


def unguarded_graphql_first_connection_lines(text: str) -> list[int]:
    lines: list[int] = []
    for literal in lua_string_literals(text):
        if "first" not in literal.content or "{" not in literal.content:
            continue
        for match in GRAPHQL_FIRST_CONNECTION_RE.finditer(literal.content):
            open_index = match.end() - 1
            close_index = matching_graphql_brace(literal.content, open_index)
            if close_index is None:
                continue
            selection_body = literal.content[match.end() : close_index]
            if not graphql_connection_has_truncation_guard(selection_body):
                lines.append(literal.line + literal.content.count("\n", 0, match.start()))
    return lines


def hidden_text_string_char_lines(text: str) -> list[int]:
    stripped = strip_lua_comments_and_strings(text)
    lines: list[int] = []
    for match in LONG_STRING_CHAR_RE.finditer(stripped):
        numeric_args = NUMERIC_ARG_RE.findall(match.group("args"))
        if len(numeric_args) >= HIDDEN_TEXT_STRING_CHAR_ARG_MIN:
            lines.append(text.count("\n", 0, match.start()) + 1)
    return lines


def unclassified_error_call_lines(text: str) -> list[int]:
    stripped = strip_lua_comments_and_strings(text)
    lines: list[int] = []
    for match in ERROR_CALL_STRING_RE.finditer(text):
        if not is_unmasked_range(text, stripped, match.start(), match.start("quote")):
            continue
        message = match.group("message")
        if not ERROR_CLASS_PREFIX_RE.match(message):
            lines.append(text.count("\n", 0, match.start()) + 1)
    return lines


def check_line_limit(root: Path, violations: list[str]) -> None:
    for scan_root_name in ("packages", "scripts"):
        scan_root = root / scan_root_name
        if not scan_root.exists():
            continue
        for path in sorted(scan_root.rglob("*")):
            if not path.is_file() or path.suffix not in SOURCE_SUFFIXES:
                continue
            count = line_count(path)
            if count > LINE_LIMIT:
                add(
                    violations,
                    "G1",
                    f"{rel(root, path)} has {count} lines; limit is {LINE_LIMIT}",
                )


def package_dirs(root: Path) -> list[Path]:
    packages = root / "packages"
    if not packages.exists():
        return []
    return [path for path in sorted(packages.iterdir()) if path.is_dir()]


def test_files(pkg: Path) -> list[Path]:
    tests = pkg / "tests"
    if not tests.exists():
        return []
    return [path for path in sorted(tests.rglob("*.lua")) if path.is_file()]


def test_name(match: re.Match[str]) -> str:
    for group in ("bare", "bracket", "field", "field_bracket"):
        try:
            name = match.group(group)
        except IndexError:
            continue
        if name is not None:
            return name
    raise ValueError("test name pattern did not capture a test name")


def has_table_key_prefix(text: str, start: int) -> bool:
    line_start = text.rfind("\n", 0, start) + 1
    cursor = start - 1
    while cursor >= line_start and text[cursor].isspace():
        cursor -= 1
    if cursor < line_start:
        return True
    return text[cursor] in ("{", ",", ";")


def matched_test_names(text: str, pattern: re.Pattern[str]) -> list[str]:
    stripped = strip_lua_comments_and_strings(text)
    return [
        test_name(match)
        for match in pattern.finditer(stripped)
        if has_table_key_prefix(stripped, match.start())
    ]


def assignment_match_is_test_entry(text: str, match: re.Match[str]) -> bool:
    if match.group("field") is not None or match.group("field_bracket") is not None:
        return True
    return has_table_key_prefix(text, match.start())


def matched_test_assignment_names(text: str) -> list[str]:
    stripped = strip_lua_comments_and_strings(text)
    return [
        test_name(match)
        for match in TEST_ASSIGN_RE.finditer(stripped)
        if assignment_match_is_test_entry(stripped, match)
    ]


def function_sugar_match_is_test_entry(text: str, match: re.Match[str]) -> bool:
    line_start = text.rfind("\n", 0, match.start()) + 1
    prefix = text[line_start : match.start()].strip()
    if prefix == "local":
        return False
    if match.group("field") is not None:
        return True
    return prefix == ""


def matched_test_function_sugar_names(text: str) -> list[str]:
    stripped = strip_lua_comments_and_strings(text)
    return [
        test_name(match)
        for match in TEST_FUNCTION_SUGAR_RE.finditer(stripped)
        if function_sugar_match_is_test_entry(stripped, match)
    ]


def test_function_names(text: str) -> list[str]:
    return matched_test_names(text, TEST_DEF_RE)


def test_assignment_names(text: str) -> list[str]:
    return matched_test_assignment_names(text) + matched_test_function_sugar_names(text)


def duplicate_test_names(names: list[str]) -> list[str]:
    seen: set[str] = set()
    duplicates: set[str] = set()
    for name in names:
        if name in seen:
            duplicates.add(name)
        else:
            seen.add(name)
    return sorted(duplicates)


def is_unmasked_range(text: str, stripped: str, start: int, end: int) -> bool:
    for index in range(start, end):
        if text[index] in (" ", "\n"):
            continue
        if stripped[index] != text[index]:
            return False
    return True


def require_module(match: re.Match[str]) -> str:
    module = match.group("quoted")
    if module is not None:
        return module
    return match.group("long")


def require_string_span(match: re.Match[str]) -> tuple[int, int]:
    if match.group("quoted") is not None:
        return match.start("quote"), match.end("quoted") + 1
    return match.start("long_literal"), match.end("long_literal")


def require_parens_are_balanced(match: re.Match[str]) -> bool:
    return match.group("open_parens").count("(") == len(match.group("close_parens"))


def is_code_require_match(text: str, stripped: str, match: re.Match[str]) -> bool:
    string_start, string_end = require_string_span(match)
    return require_parens_are_balanced(match) and is_unmasked_range(
        text,
        stripped,
        match.start(),
        string_start,
    ) and is_unmasked_range(
        text,
        stripped,
        string_end,
        match.end(),
    )


def required_modules(text: str) -> list[str]:
    stripped = strip_lua_comments_and_strings(text)
    return [
        require_module(match)
        for match in TEST_REQUIRE_RE.finditer(text)
        if is_code_require_match(text, stripped, match)
    ]


def module_path(tests_dir: Path, module: str) -> Path:
    return tests_dir.joinpath(*module.split(".")).with_suffix(".lua")


def module_name(tests_dir: Path, path: Path) -> str:
    return path.relative_to(tests_dir).with_suffix("").as_posix().replace("/", ".")


def check_test_shape(root: Path, violations: list[str], warnings: list[str]) -> None:
    for pkg in package_dirs(root):
        for path in test_files(pkg):
            name = path.name
            is_test = name.endswith("_test.lua")
            is_helper = name.endswith("_helpers.lua")
            if not is_test and not is_helper:
                add(
                    violations,
                    "G2",
                    f"{rel(root, path)} must be named *_test.lua or *_helpers.lua",
                )
                continue

            text = read_text(path)
            runnable_names = test_function_names(text)
            assigned_names = test_assignment_names(text)
            if is_test:
                duplicates = duplicate_test_names(assigned_names)
                for duplicate in duplicates:
                    add(
                        violations,
                        "G2",
                        f"{rel(root, path)} defines duplicate top-level test name: {duplicate}",
                    )
                if not runnable_names:
                    add(
                        warnings,
                        "G2",
                        f"{rel(root, path)} has no best-effort test_<name> = function lint match; engine G5 is authoritative",
                    )
            if is_helper and assigned_names:
                add(
                    violations,
                    "G2",
                    f"{rel(root, path)} is a helper but defines test entries: {', '.join(sorted(set(assigned_names)))}",
                )


def check_helper_reachability(root: Path, violations: list[str]) -> None:
    for pkg in package_dirs(root):
        tests_dir = pkg / "tests"
        files = test_files(pkg)
        if not files:
            continue

        requires_by_file: dict[Path, list[str]] = {}
        file_by_module = {module_name(tests_dir, path): path for path in files}
        for path in files:
            requires_by_file[path] = required_modules(read_text(path))

        for path, modules in requires_by_file.items():
            for module in modules:
                target = module_path(tests_dir, module)
                if not target.exists():
                    add(
                        violations,
                        "G3",
                        f"{rel(root, path)} requires tests.{module}, but {rel(root, target)} does not exist",
                    )
                    continue
                if target.name.endswith("_test.lua"):
                    add(
                        violations,
                        "G3",
                        f"{rel(root, path)} must not require test module tests.{module}",
                    )

        reachable: set[Path] = set()
        pending = [path for path in files if path.name.endswith("_test.lua")]
        while pending:
            path = pending.pop()
            if path in reachable:
                continue
            reachable.add(path)
            for module in requires_by_file.get(path, []):
                target = file_by_module.get(module)
                if target is not None and target not in reachable:
                    pending.append(target)

        for helper in sorted(path for path in files if path.name.endswith("_helpers.lua")):
            module = module_name(tests_dir, helper)
            if helper not in reachable:
                add(
                    violations,
                    "G3",
                    f"{rel(root, helper)} is not reachable from any *_test.lua as tests.{module}",
                )


def check_graphql_connection_guards(root: Path, warnings: list[str]) -> None:
    packages = root / "packages"
    if not packages.exists():
        return
    for path in sorted(packages.rglob("*.lua")):
        if not path.is_file():
            continue
        for line in unguarded_graphql_first_connection_lines(read_text(path)):
            add(
                warnings,
                "G4",
                f"{rel(root, path)}:{line} GraphQL first:N connection lacks a truncation guard; possible fail-open; explicitly detect truncation or fail closed",
            )


def check_hidden_text_string_char(root: Path, warnings: list[str]) -> None:
    packages = root / "packages"
    if not packages.exists():
        return
    for path in sorted(packages.rglob("*.lua")):
        if not path.is_file() or "tests" in path.relative_to(packages).parts:
            continue
        for line in hidden_text_string_char_lines(read_text(path)):
            add(
                warnings,
                "G6",
                f"{rel(root, path)}:{line} string.char call uses a long numeric byte sequence; use a plain English literal instead",
            )


def check_error_class_prefixes(root: Path, warnings: list[str]) -> None:
    packages = root / "packages"
    if not packages.exists():
        return
    for path in sorted(packages.rglob("*.lua")):
        if not path.is_file() or "tests" in path.relative_to(packages).parts:
            continue
        for line in unclassified_error_call_lines(read_text(path)):
            add(
                warnings,
                "G7",
                f"{rel(root, path)}:{line} production error(...) string lacks a greppable class prefix",
            )


def main() -> int:
    root = repo_root()
    violations: list[str] = []
    warnings: list[str] = []

    check_line_limit(root, violations)
    check_test_shape(root, violations, warnings)
    check_helper_reachability(root, violations)
    check_graphql_connection_guards(root, warnings)
    check_hidden_text_string_char(root, warnings)
    check_error_class_prefixes(root, warnings)

    for warning in warnings:
        print(f"warning: {warning}", file=sys.stderr)

    if violations:
        print("repository check failed:", file=sys.stderr)
        for violation in violations:
            print(f"  {violation}", file=sys.stderr)
        return 1

    print("OK: repository checks passed")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
