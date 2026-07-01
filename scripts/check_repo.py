#!/usr/bin/env python3
"""Hermetic repository guards for fkst packages."""

from __future__ import annotations

import re
import sys
import os, base64, binascii, subprocess
from dataclasses import dataclass
from pathlib import Path
import check_repo_config, check_repo_content_truncation, check_repo_dedup, check_repo_gh_git_adapter as gh_git_adapter, check_repo_ingress, check_repo_integration_coverage, check_repo_namespaced_queue, check_repo_perm, check_repo_producer_liveness, check_repo_saga_head, check_repo_shell_out_to_self, check_repo_std_dependency_model, ratchet_base
LINE_LIMIT = 1000
# Soft-split threshold is 900 (LINE_LIMIT - 100): warn early so files split at ~900 by
# stable responsibility rather than being forced at the 1000-line hard limit, where any
# small later change (even one new require alias) tips them over and blocks unrelated PRs.
LINE_WARNING_MARGIN = 100
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
NUMERIC_ARG_RE = re.compile(r"(?:^|,)\s*(?:0x[0-9A-Fa-f]+|\d+)\s*(?=,|\Z)")
HIDDEN_TEXT_STRING_CHAR_ARG_MIN = 6
ERROR_CALL_STRING_RE = re.compile(r"\berror\s*\(\s*(?P<quote>['\"])(?P<message>[^'\"]*)(?P=quote)")
ERROR_CLASS_PREFIX_RE = re.compile(r"^[a-z0-9][a-z0-9-]*: [a-z0-9][a-z0-9-]*:")
HELPER_STRING_ARG_RE = re.compile(
    r"\b(?P<func>(?:[A-Za-z_][A-Za-z0-9_]*\s*\.\s*)?[A-Za-z_][A-Za-z0-9_]*)"
    r"\s*\(\s*(?P<quote>[\"'])"
)
GH_RATE_POOL_FUNCTION_RE = re.compile(
    r"\bfunction\b[^\n]*\bgh_rate_pool\b|\bgh_rate_pool\b\s*=\s*function\b"
)
GH_RATE_POOL_SIZING_FIELD_RE = re.compile(r"\b(?:burst|refill_per_(?:hour|minute))\b")
OWNERSHIP_GATE_RE = re.compile(r"(?ms)^\s*function\s+M\s*\.\s*verify_pr_review_issue_claim\s*\([^)]*\).*?(?=^\s*function\s+M\s*\.|\Z)")
OWNERSHIP_GATE_CLAIMS_PATH = Path("libraries/devloop/claims.lua")
# Declaration presence + valid value moved to the engine manifest schema
# (`persistence_class` in fkst.toml + the `engine.persistence-class` conformance
# check, the single authority). This regex reads that authoritative field only to
# drive the saga-recovery-token guard below (a not-yet-promoted follow-up).
PERSISTENCE_CLASS_RE = re.compile(
    r"(?m)^\s*persistence_class\s*=\s*(?P<quote>[\"'])(?P<class>[A-Za-z0-9_]+)(?P=quote)"
)
SAGA_RECOVERY_TOKENS = ("fkst:github-devloop:state:v1", "current_entity_state", "restart_completeness", "transition_status", "versioned_transition_status", "cyclic_transition_status")
HEX_LITERAL_RE = re.compile(r"[0-9A-Fa-f]+\Z")
BASE64_LITERAL_RE = re.compile(r"[A-Za-z0-9+/]+={0,2}\Z")
BYTE_ESCAPE_RE = re.compile(r"\\x[0-9A-Fa-f]{2}|\\[0-9]{1,3}|\\u\{[0-9A-Fa-f]+\}")
ENCODED_LITERAL_MIN_BYTES = 6
@dataclass(frozen=True)
class LuaStringLiteral:
    line: int
    content: str

def repo_root() -> Path:
    return check_repo_config.default_project_root()

def allowlist_path(root: Path, relpath: str, allowlist_dir: Path | None = None) -> Path:
    return check_repo_config.allowlist_path(root, allowlist_dir, relpath)

def rel(root: Path, path: Path) -> str:
    for packages_view in package_roots(root):
        try:
            return "packages/" + path.relative_to(packages_view).as_posix()
        except ValueError:
            pass
    return path.relative_to(root).as_posix()

def read_text(path: Path) -> str: return path.read_text(encoding="utf-8")
def package_roots(root: Path) -> list[Path]: return check_repo_config.package_roots(root)
def packages_root(root: Path) -> Path: return check_repo_config.package_root(root)
def line_count(path: Path) -> int: return len(read_text(path).splitlines())
def add(violations: list[str], rule: str, message: str) -> None: violations.append(f"{rule}: {message}")

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


def unguarded_rest_per_page_lines(text: str) -> list[int]:
    lines: list[int] = []
    source_lines = text.splitlines()
    for index, line in enumerate(source_lines):
        if "per_page=100" not in line:
            continue
        window = "\n".join(source_lines[max(0, index - 12) : index + 7])
        raw_unpaginated_line = "gh api" in line and "--paginate" not in line and "%-%-paginate" not in line
        if raw_unpaginated_line or not ((("--paginate" in window or "%-%-paginate" in window) and ("gh api" in window or re.search(r"['\"]gh['\"]\s*,\s*['\"]api['\"]", window) is not None)) or re.search(r"(?:[\.:]\s*|\b)[A-Za-z0-9_]*paginate[A-Za-z0-9_]*\s*\(", window) is not None):
            lines.append(index + 1)
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


def looks_like_decode_helper(func: str) -> bool:
    normalized = re.sub(r"\s+", "", func).lower()
    base_name = normalized.rsplit(".", 1)[-1]
    if base_name in {"h", "b", "u", "hex", "base64", "b64", "bytes", "byte"}:
        return True
    helper_tokens = (
        "decode",
        "fromhex",
        "from_hex",
        "unhex",
        "unescape",
    )
    return any(token in normalized for token in helper_tokens)


def byte_escape_count(content: str) -> int:
    return len(BYTE_ESCAPE_RE.findall(content))


def is_printable_utf8(data: bytes) -> bool:
    if len(data) < ENCODED_LITERAL_MIN_BYTES:
        return False
    try:
        decoded = data.decode("utf-8")
    except UnicodeDecodeError:
        return False
    if not decoded:
        return False
    printable = sum(1 for char in decoded if char.isprintable() or char in "\n\r\t")
    return printable / len(decoded) >= 0.8


def encoded_literal_kind(content: str) -> str | None:
    if (
        len(content) >= ENCODED_LITERAL_MIN_BYTES * 2
        and len(content) % 2 == 0
        and HEX_LITERAL_RE.fullmatch(content) is not None
    ):
        return "hex"

    if byte_escape_count(content) >= ENCODED_LITERAL_MIN_BYTES:
        return "byte-escape"

    if (
        len(content) >= ENCODED_LITERAL_MIN_BYTES * 2
        and len(content) % 4 == 0
        and BASE64_LITERAL_RE.fullmatch(content) is not None
    ):
        try:
            decoded = base64.b64decode(content, validate=True)
        except (binascii.Error, ValueError):
            decoded = b""
        if is_printable_utf8(decoded):
            return "base64"

    return None


def hidden_text_encoded_literal_lines(text: str) -> list[int]:
    stripped = strip_lua_comments_and_strings(text)
    lines: list[int] = hidden_text_string_char_lines(text)
    for match in HELPER_STRING_ARG_RE.finditer(text):
        quote_start = match.start("quote")
        if not is_unmasked_range(text, stripped, match.start(), quote_start):
            continue
        string_end = end_of_quoted_string(text, quote_start)
        if string_end > len(text) or text[string_end - 1] != match.group("quote"):
            continue
        if not looks_like_decode_helper(match.group("func")):
            continue
        content = text[quote_start + 1 : string_end - 1]
        if encoded_literal_kind(content) is not None:
            lines.append(text.count("\n", 0, match.start()) + 1)
    return sorted(set(lines))


def ownership_gate_defaulting_bot_login_lines(text: str) -> list[int]:
    stripped = strip_lua_comments_and_strings(text)
    gate = OWNERSHIP_GATE_RE.search(stripped)
    return [] if gate is None else [
        text.count("\n", 0, gate.start() + m.start()) + 1
        for m in re.finditer(r"\bM\s*\.\s*trusted_bot_login\s*\(", gate.group(0))]


def line_warning_threshold() -> int:
    raw = os.environ.get("FKST_G1_LINE_WARNING_THRESHOLD")
    if raw is None or raw == "":
        return LINE_LIMIT - LINE_WARNING_MARGIN
    try:
        threshold = int(raw)
    except ValueError:
        return LINE_LIMIT - LINE_WARNING_MARGIN
    if threshold < 1:
        return LINE_LIMIT - LINE_WARNING_MARGIN
    return threshold


def check_line_limit(root: Path, violations: list[str], warnings: list[str]) -> None:
    warning_threshold = line_warning_threshold()
    for scan_root in (*package_roots(root), root / "scripts"):
        if not scan_root.exists():
            continue
        for path in sorted(scan_root.rglob("*")):
            if not path.is_file() or path.suffix not in SOURCE_SUFFIXES:
                continue
            count = line_count(path)
            if count > LINE_LIMIT:
                add(violations, "G1", f"{rel(root, path)} has {count} lines; limit is {LINE_LIMIT}")
            elif count >= warning_threshold:
                add(warnings, "G1", f"{rel(root, path)} has {count} lines; warning threshold is {warning_threshold}; hard limit is {LINE_LIMIT}")


def package_dirs(root: Path) -> list[Path]:
    return [
        path
        for packages in package_roots(root)
        if packages.exists()
        for path in sorted(packages.iterdir())
        if path.is_dir()
    ]


def package_lua_files(root: Path) -> list[tuple[Path, Path]]:
    return [
        (packages, path)
        for packages in package_roots(root)
        if packages.exists()
        for path in sorted(packages.rglob("*.lua"))
        if path.is_file()
    ]


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
    for _packages, path in package_lua_files(root):
        for line in unguarded_graphql_first_connection_lines(read_text(path)):
            add(
                warnings,
                "G4",
                f"{rel(root, path)}:{line} GraphQL first:N connection lacks a truncation guard; possible fail-open; explicitly detect truncation or fail closed",
            )


def check_rest_pagination_guards(root: Path, warnings: list[str]) -> None:
    for _packages, path in package_lua_files(root):
        for line in unguarded_rest_per_page_lines(read_text(path)):
            add(
                warnings,
                "G5",
                f"{rel(root, path)}:{line} REST per_page=100 read lacks gh api --paginate; possible fail-open truncation",
            )


def check_hidden_text_encoded_literals(root: Path, violations: list[str]) -> None:
    for packages, path in package_lua_files(root):
        if not path.is_file() or "tests" in path.relative_to(packages).parts:
            continue
        for line in hidden_text_encoded_literal_lines(read_text(path)):
            add(
                violations,
                "G6",
                f"{rel(root, path)}:{line} hidden text uses an encoded literal decode helper; use a plain source literal instead",
            )


def gh_rate_pool_sizing_lines(text: str) -> list[int]:
    stripped = strip_lua_comments_and_strings(text)
    lines: list[int] = []
    in_gh_rate_pool = False
    for index, line in enumerate(stripped.splitlines(), start=1):
        if not in_gh_rate_pool and GH_RATE_POOL_FUNCTION_RE.search(line):
            in_gh_rate_pool = True

        if in_gh_rate_pool and GH_RATE_POOL_SIZING_FIELD_RE.search(line):
            lines.append(index)

        if in_gh_rate_pool and re.match(r"^\s*end\s*[,;]?\s*$", line):
            in_gh_rate_pool = False
    return lines


def check_gh_rate_pool_sizing(root: Path, violations: list[str]) -> None:
    for packages, path in package_lua_files(root):
        if not path.is_file() or "tests" in path.relative_to(packages).parts:
            continue
        for line in gh_rate_pool_sizing_lines(read_text(path)):
            add(
                violations,
                "G7",
                f"{rel(root, path)}:{line} gh rate pool sizing belongs to FKST_RATE_POOL_GH host posture; package code may declare only the pool name",
            )


def check_error_class_prefixes(root: Path, warnings: list[str]) -> None:
    for packages, path in package_lua_files(root):
        if not path.is_file() or "tests" in path.relative_to(packages).parts:
            continue
        for line in unclassified_error_call_lines(read_text(path)):
            add(
                warnings,
                "G7",
                f"{rel(root, path)}:{line} production error(...) string lacks a greppable class prefix",
            )


def check_ownership_gate_claim_owner(root: Path, violations: list[str]) -> None:
    path = root / OWNERSHIP_GATE_CLAIMS_PATH
    if not path.exists():
        add(violations, "G8", f"{OWNERSHIP_GATE_CLAIMS_PATH.as_posix()} is missing; ownership gate guard cannot run")
        return
    for line in ownership_gate_defaulting_bot_login_lines(read_text(path)):
        add(
            violations,
            "G8",
            f"{rel(root, path)}:{line} verify_pr_review_issue_claim must use claim_owner(), not the defaulting trusted_bot_login() getter",
        )


def package_persistence_class(pkg: Path) -> str | None:
    fkst_toml = pkg / "fkst.toml"
    if not fkst_toml.exists():
        return None
    match = PERSISTENCE_CLASS_RE.search(read_text(fkst_toml))
    if match is None:
        return None
    return match.group("class")


def check_persistence_classes(root: Path, violations: list[str]) -> None:
    # Declaration presence and valid value are enforced by the engine
    # `engine.persistence-class` conformance check (the single authority, read
    # from the typed `persistence_class` manifest field). This ratchet retains
    # only the saga-recovery-token guard for non-saga packages — a follow-up
    # promotion (gating the marker primitives on the saga_recovery capability).
    for pkg in package_dirs(root):
        declared = package_persistence_class(pkg)
        if declared is None or declared == "saga":
            continue
        for path in sorted(pkg.rglob("*.lua")):
            if not path.is_file() or "tests" in path.relative_to(pkg).parts:
                continue
            text = read_text(path)
            for token in SAGA_RECOVERY_TOKENS:
                if token in text:
                    add(
                        violations,
                        "G8",
                        f"{rel(root, path)} uses saga recovery token {token!r} but {pkg.name} is {declared}",
                    )


REQUIRE_RE = re.compile(
    r"""\brequire\s*(?:\(\s*)?(?:"([A-Za-z0-9_.\-]+)"|'([A-Za-z0-9_.\-]+)'|\[(=*)\[([A-Za-z0-9_.\-]+)\]\3\])"""
)
SAGA_REQUIRE_RE = re.compile(r"""\brequire\s*(?:\(\s*)?["']workflow\.saga["']""")
SAGA_DEPARTMENT_RE = re.compile(r"\.\s*department\s*[({]")
FREE_FORM_PIPELINE_RE = re.compile(r"(?m)^\s*(?:function\s+pipeline\s*\(|pipeline\s*=\s*function\b)")


def cross_package_require_names(
    source: str, package_names: set[str], current_pkg: str
) -> list[str]:
    """Top-level require names in `source` that name a sibling package."""
    hits: set[str] = set()
    stripped = strip_lua_comments_and_strings(source)
    for match in REQUIRE_RE.finditer(source):
        group = 1 if match.group(1) is not None else 2 if match.group(2) is not None else 4
        string_start = match.start(group) - 1
        string_end = match.end(group) + 1
        if group == 4:
            string_end += 1 + len(match.group(3))
        if not (is_unmasked_range(source, stripped, match.start(), string_start) and is_unmasked_range(source, stripped, string_end, match.end())):
            continue
        name = next(group for group in (match.group(1), match.group(2), match.group(4)) if group is not None)
        top = name.split(".")[0]
        if top in package_names and top != current_pkg:
            hits.add(top)
    return sorted(hits)


def check_cross_package_require(root: Path, violations: list[str]) -> None:
    pkgs = package_dirs(root)
    names = {pkg.name for pkg in pkgs}
    for pkg in pkgs:
        for path in sorted(pkg.rglob("*.lua")):
            if not path.is_file():
                continue
            parts = path.relative_to(pkg).parts
            # Skip package-local external library symlink trees if a host repo has them.
            if parts and parts[0] in {"std", "libraries"}:
                continue
            for name in cross_package_require_names(read_text(path), names, pkg.name):
                add(
                    violations,
                    "G9",
                    f"{rel(root, path)} peer cross-package require of {name!r}; share via workspace libraries (peer cross-package require is forbidden)",
                )

def check_gh_git_adapter_ratchet(root: Path, violations: list[str], allowlist_dir: Path | None = None) -> None:
    sources = {}
    for packages in package_roots(root):
        sources.update(gh_git_adapter.sources(root, packages, read_text, rel))
    allowlist = gh_git_adapter.load_allowlist(allowlist_path(root, gh_git_adapter.ALLOWLIST, allowlist_dir))
    for message in gh_git_adapter.ratchet_messages(sources, allowlist, lua_string_literals):
        add(violations, "G-ADAPTER", message)

def check_shell_out_to_self_ratchet(root: Path, violations: list[str], allowlist_dir: Path | None = None) -> None:
    current = check_repo_shell_out_to_self.sites(root, package_roots(root), read_text, rel, strip_lua_comments_and_strings, lua_string_literals)
    allowlist = check_repo_shell_out_to_self.load_allowlist(
        allowlist_path(root, check_repo_shell_out_to_self.ALLOWLIST, allowlist_dir)
    )
    for message in check_repo_shell_out_to_self.ratchet_messages(current, allowlist):
        add(violations, "G-SHELL-OUT-TO-SELF", message)

def check_code_dedup_ratchet(root: Path, violations: list[str], allowlist_dir: Path | None = None, enforce_base: bool = True) -> None:
    source_map = {}
    for packages in package_roots(root):
        source_map.update(check_repo_dedup.sources(root, packages, read_text, rel))
    allowlist = check_repo_dedup.load_allowlist(allowlist_path(root, check_repo_dedup.ALLOWLIST, allowlist_dir))
    base_status, base_allowlist = check_repo_dedup.allowlist_at_dev_base(root) if enforce_base else ("absent", None)
    if base_status == "unresolved": add(violations, "G-DEDUP", "cannot resolve dev base allowlist to enforce shrink-only ratchet; ensure CI provides the dev ref")
    for message in check_repo_dedup.ratchet_messages(source_map, allowlist, base_allowlist):
        add(violations, "G-DEDUP", message)

def check_std_dependency_model(root: Path, violations: list[str], warnings: list[str]) -> None: check_repo_std_dependency_model.check_std_dependency_model(root, violations, warnings, packages=package_dirs(root), read_text=read_text, rel=rel, add=add, strip_lua_comments_and_strings=strip_lua_comments_and_strings, is_unmasked_range=is_unmasked_range)
def check_no_permission_control(root: Path, violations: list[str]) -> None: check_repo_perm.check_no_permission_control(root, violations, read_text=read_text, rel=rel)

def is_saga_handler_source(source: str) -> bool:
    return SAGA_REQUIRE_RE.search(source) is not None and SAGA_DEPARTMENT_RE.search(strip_lua_comments_and_strings(source)) is not None

def saga_handler_ratchet_violations(sources: dict[str, str], allowlist: set[str], base_allowlist: set[str] | None = None) -> list[str]:
    violations: list[str] = []
    for path, source in sorted(sources.items()):
        saga_shaped = is_saga_handler_source(source)
        if saga_shaped and path in allowlist:
            violations.append(f"G10: {path} saga-shaped department remains on saga-handler allowlist; remove it")
        if saga_shaped and FREE_FORM_PIPELINE_RE.search(strip_lua_comments_and_strings(source)) is not None:
            violations.append(f"G10: {path} saga-shaped department still defines free-form top-level pipeline")
        if not saga_shaped and path not in allowlist:
            violations.append(f"G10: {path} free-form department not on saga-handler allowlist; migrate to workflow.saga.department or (only for pre-existing) keep listed")
    for path in sorted(allowlist - set(sources)):
        violations.append(f"G10: {path} listed in saga-handler allowlist but does not exist")
    if base_allowlist is not None:
        violations.extend(f"G10: {path} grows saga-handler allowlist relative to dev; migrate instead" for path in sorted(allowlist - base_allowlist))
    return violations

def saga_allowlist_at_dev_base(root: Path) -> tuple[str, set[str] | None]:
    try:
        status, shown = ratchet_base.file_at_base(root, "migration/saga-handler.allowlist")
        if status != "present":
            return status, None
        assert shown is not None
        return "present", {line.strip() for line in shown.splitlines() if line.strip() and not line.lstrip().startswith("#")}
    except Exception:
        return "unresolved", None

def check_saga_handler_ratchet(root: Path, violations: list[str], warnings: list[str], allowlist_dir: Path | None = None, enforce_base: bool = True) -> None:
    allow_path = allowlist_path(root, "migration/saga-handler.allowlist", allowlist_dir)
    allowlist = set() if not allow_path.exists() else {line.strip() for line in read_text(allow_path).splitlines() if line.strip() and not line.lstrip().startswith("#")}
    sources = {rel(root, path): read_text(path) for packages in package_roots(root) for path in sorted(packages.glob("*/departments/*/main.lua")) if path.is_file()}
    base_status, base_allowlist = saga_allowlist_at_dev_base(root) if enforce_base else ("absent", None)
    if base_status == "unresolved": violations.append("G10: cannot resolve dev base allowlist to enforce shrink-only ratchet; ensure CI provides the dev ref")
    violations.extend(saga_handler_ratchet_violations(sources, allowlist, base_allowlist))

def main(argv: list[str] | None = None) -> int:
    config = check_repo_config.parse_args(argv); violations: list[str] = []; warnings: list[str] = []
    __import__("check_repo_runner").run(sys.modules[__name__], config, violations, warnings)
    for warning in warnings: print(f"warning: {warning}", file=sys.stderr)
    if violations:
        print("repository check failed:", file=sys.stderr)
        for violation in violations: print(f"  {violation}", file=sys.stderr)
        return 1
    print("OK: repository checks passed")
    return 0
if __name__ == "__main__": raise SystemExit(main())
