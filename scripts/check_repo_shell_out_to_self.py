#!/usr/bin/env python3
"""Detect package Lua shell-outs to the framework binary."""

from __future__ import annotations

import re
from pathlib import Path
from typing import Callable, Iterable

ALLOWLIST = "migration/shell-out-to-self.allowlist"
# This ratchet is best-effort DETECT coverage. The true PREVENT follow-up is the
# filed engine capability boundary where package code never receives the engine
# binary path or BIN at runtime, so package code cannot shell out to the framework.
ENGINE_BASE_NAMES = {"BIN", "bin", "framework_bin"}
ARGV_EXEC_NAMES = {"exec_argv", "run_argv"}
SYNC_EXEC_NAMES = {"exec_sync", "run_sync"}
EXEC_CALL_RE = re.compile(r"\b(?P<name>exec_argv|exec_sync|run_argv|run_sync)\s*\(")
ANY_CALL_RE = re.compile(r"\b(?P<name>[A-Za-z_][A-Za-z0-9_]*)\s*\(")
ASSIGN_RE = re.compile(r"\b(?:local\s+)?(?P<name>[A-Za-z_][A-Za-z0-9_]*)\s*=")
IDENT_RE = re.compile(r"^[A-Za-z_][A-Za-z0-9_]*$")


def long_bracket_at(text: str, index: int) -> tuple[int, str] | None:
    if index >= len(text) or text[index] != "[":
        return None
    cursor = index + 1
    while cursor < len(text) and text[cursor] == "=":
        cursor += 1
    if cursor >= len(text) or text[cursor] != "[":
        return None
    return cursor - index + 1, "]" + ("=" * (cursor - index - 1)) + "]"


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


def end_of_long_bracket(text: str, body_start: int, closer: str) -> int:
    close_start = text.find(closer, body_start)
    return len(text) if close_start == -1 else close_start + len(closer)


def mask_span(chars: list[str], start: int, end: int) -> None:
    for index in range(start, end):
        if chars[index] != "\n":
            chars[index] = " "


def strip_lua_comments(text: str) -> str:
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
        if text[cursor] in ("'", '"'):
            cursor = end_of_quoted_string(text, cursor)
            continue
        bracket = long_bracket_at(text, cursor)
        if bracket is not None:
            opener_len, closer = bracket
            cursor = end_of_long_bracket(text, cursor + opener_len, closer)
            continue
        cursor += 1
    return "".join(chars)


def skip_ws(text: str, cursor: int) -> int:
    while cursor < len(text) and text[cursor].isspace():
        cursor += 1
    return cursor


def skip_lua_string(text: str, cursor: int) -> int:
    if cursor >= len(text):
        return cursor
    if text[cursor] in ("'", '"'):
        return end_of_quoted_string(text, cursor)
    bracket = long_bracket_at(text, cursor)
    if bracket is not None:
        opener_len, closer = bracket
        return end_of_long_bracket(text, cursor + opener_len, closer)
    return cursor + 1


def find_matching(text: str, start: int, opener: str, closer: str) -> int:
    depth = 0
    cursor = start
    while cursor < len(text):
        if text[cursor] in ("'", '"') or long_bracket_at(text, cursor) is not None:
            cursor = skip_lua_string(text, cursor)
            continue
        if text[cursor] == opener:
            depth += 1
        elif text[cursor] == closer:
            depth -= 1
            if depth == 0:
                return cursor + 1
        cursor += 1
    return len(text)


def expression_end(text: str, start: int) -> int:
    cursor = skip_ws(text, start)
    if cursor >= len(text):
        return cursor
    if text[cursor] == "{":
        return find_matching(text, cursor, "{", "}")
    depth = 0
    while cursor < len(text):
        if text[cursor] in ("'", '"') or long_bracket_at(text, cursor) is not None:
            cursor = skip_lua_string(text, cursor)
            continue
        char = text[cursor]
        if char in "({[":
            depth += 1
        elif char in ")}]":
            if depth == 0:
                return cursor
            depth -= 1
        elif depth == 0 and char in ",\n":
            return cursor
        cursor += 1
    return cursor


def parse_literal_content(expr: str) -> str | None:
    text = expr.strip()
    if not text:
        return None
    if text[0] in ("'", '"'):
        end = end_of_quoted_string(text, 0)
        if end <= len(text) and end > 1 and text[end - 1] == text[0]:
            return text[1 : end - 1]
        return text[1:end]
    bracket = long_bracket_at(text, 0)
    if bracket is None:
        return None
    opener_len, closer = bracket
    body_start = opener_len
    close_start = text.find(closer, body_start)
    body_end = len(text) if close_start == -1 else close_start
    return text[body_start:body_end]


def strip_outer_parens(expr: str) -> str:
    text = expr.strip()
    while text.startswith("(") and find_matching(text, 0, "(", ")") == len(text):
        text = text[1:-1].strip()
    return text


def split_top_level_args(text: str) -> list[str]:
    args: list[str] = []
    start = 0
    depth = 0
    cursor = 0
    while cursor < len(text):
        if text[cursor] in ("'", '"') or long_bracket_at(text, cursor) is not None:
            cursor = skip_lua_string(text, cursor)
            continue
        char = text[cursor]
        if char in "({[":
            depth += 1
        elif char in ")}]":
            depth = max(depth - 1, 0)
        elif char == "," and depth == 0:
            args.append(text[start:cursor])
            start = cursor + 1
        cursor += 1
    args.append(text[start:])
    return args


def top_level_table_field_expr(expr: str, field: str) -> str | None:
    text = strip_outer_parens(expr)
    start = skip_ws(text, 0)
    if start >= len(text) or text[start] != "{":
        return None
    end = find_matching(text, start, "{", "}")
    body = text[start + 1 : end - 1]
    for arg in split_top_level_args(body):
        candidate = arg.strip()
        if not candidate:
            continue
        bare = re.match(r"^(?P<key>[A-Za-z_][A-Za-z0-9_]*)\s*=(?P<value>.*)\Z", candidate, re.DOTALL)
        if bare is not None and bare.group("key") == field:
            return bare.group("value").strip()
        indexed = re.match(
            r"^\[\s*(?P<quote>['\"])(?P<key>[A-Za-z_][A-Za-z0-9_]*)"
            r"(?P=quote)\s*\]\s*=(?P<value>.*)\Z",
            candidate,
            re.DOTALL,
        )
        if indexed is not None and indexed.group("key") == field:
            return indexed.group("value").strip()
    return None


def table_head_expr(expr: str) -> str | None:
    text = strip_outer_parens(expr)
    start = skip_ws(text, 0)
    if start >= len(text) or text[start] != "{":
        return None
    end = find_matching(text, start, "{", "}")
    body = text[start + 1 : end - 1]
    for arg in split_top_level_args(body):
        candidate = arg.strip()
        if not candidate:
            continue
        indexed = re.match(r"^\[\s*1\s*\]\s*=(?P<value>.*)\Z", candidate, re.DOTALL)
        if indexed is not None:
            return indexed.group("value").strip()
        if re.match(r"^[A-Za-z_][A-Za-z0-9_]*\s*=", candidate):
            continue
        if candidate.startswith("["):
            continue
        return candidate
    return None


def literal_is_engine_binary(content: str) -> bool:
    tokens = content.strip().split()
    head = tokens[0] if tokens else content
    if re.search(r"\$(?:\{BIN\}|BIN\b)", content):
        return True
    if re.search(r"(?:^|[\s;&|()])(?:\S*/)?fkst-framework(?:$|[\s;&|()])", content):
        return True
    return (
        head == "fkst-framework"
        or head.endswith("/fkst-framework")
        or head in {"$BIN", "${BIN}"}
        or content == "fkst-framework"
        or content.endswith("/fkst-framework")
    )


def expr_mentions_engine(expr: str, engine_vars: set[str]) -> bool:
    text = strip_outer_parens(expr)
    literal = parse_literal_content(text)
    if literal is not None:
        return literal_is_engine_binary(literal)
    if "fkst-framework" in text or "$BIN" in text or "${BIN}" in text:
        return True
    if re.search(r"\bos\s*\.\s*getenv\s*\(\s*['\"]BIN['\"]\s*\)", text):
        return True
    return any(re.search(rf"\b{re.escape(name)}\b", text) for name in engine_vars)


def collect_engine_bindings(source: str, strip_lua_comments_and_strings: Callable[[str], str]) -> tuple[set[str], set[str]]:
    engine_vars = set(ENGINE_BASE_NAMES)
    argv_vars: set[str] = set()
    assignments: list[tuple[str, str]] = []
    stripped = strip_lua_comments(source)
    masked = strip_lua_comments_and_strings(stripped)
    for match in ASSIGN_RE.finditer(masked):
        expr_start = match.end()
        expr = stripped[expr_start:expression_end(stripped, expr_start)].strip()
        if expr:
            assignments.append((match.group("name"), expr))

    changed = True
    while changed:
        changed = False
        for name, expr in assignments:
            head = table_head_expr(expr)
            if head is not None and expr_mentions_engine(head, engine_vars):
                if name not in argv_vars:
                    argv_vars.add(name)
                    changed = True
                continue
            if expr_mentions_engine(expr, engine_vars):
                if name not in engine_vars:
                    engine_vars.add(name)
                    changed = True
    return engine_vars, argv_vars


def argv_expr_uses_engine_head(expr: str, engine_vars: set[str], argv_vars: set[str]) -> bool:
    text = strip_outer_parens(expr)
    if text in argv_vars:
        return True
    if IDENT_RE.fullmatch(text) and text in engine_vars:
        return True
    head = table_head_expr(text)
    if head is not None:
        return expr_mentions_engine(head, engine_vars)
    return False


def sync_expr_uses_engine_head(expr: str, engine_vars: set[str]) -> bool:
    return expr_mentions_engine(expr, engine_vars)


def first_call_arg(call_args: str) -> str:
    args = split_top_level_args(call_args)
    return args[0].strip() if args else ""


def call_arg(call_args: str, index: int) -> str:
    args = split_top_level_args(call_args)
    return args[index].strip() if index < len(args) else ""


def call_option_field(call_args: str, field: str) -> str | None:
    first_arg = first_call_arg(call_args)
    return top_level_table_field_expr(first_arg, field)


def argv_expr_from_exec_arg(expr: str) -> str | None:
    text = strip_outer_parens(expr)
    if not text:
        return None
    return top_level_table_field_expr(text, "argv") or text


def sync_expr_from_exec_arg(expr: str) -> str | None:
    text = strip_outer_parens(expr)
    if not text:
        return None
    return top_level_table_field_expr(text, "cmd") or text


def argv_expr_from_call_args(call_args: str) -> str | None:
    return argv_expr_from_exec_arg(first_call_arg(call_args))


def sync_expr_from_call_args(call_args: str) -> str | None:
    return sync_expr_from_exec_arg(first_call_arg(call_args))


def exec_function_kinds(
    expr: str,
    argv_exec_vars: set[str] | None = None,
    sync_exec_vars: set[str] | None = None,
) -> tuple[str, ...]:
    argv_names = ARGV_EXEC_NAMES if argv_exec_vars is None else argv_exec_vars
    sync_names = SYNC_EXEC_NAMES if sync_exec_vars is None else sync_exec_vars
    text = strip_outer_parens(expr)
    compact = re.sub(r"\s+", "", text)
    name = compact.split(".")[-1]
    kinds = []
    if name in argv_names:
        kinds.append("argv")
    if name in sync_names:
        kinds.append("sync")
    return tuple(kinds)


def collect_exec_function_bindings(
    source: str,
    strip_lua_comments_and_strings: Callable[[str], str],
) -> tuple[set[str], set[str]]:
    argv_exec_vars = set(ARGV_EXEC_NAMES)
    sync_exec_vars = set(SYNC_EXEC_NAMES)
    assignments: list[tuple[str, str]] = []
    stripped = strip_lua_comments(source)
    masked = strip_lua_comments_and_strings(stripped)
    for match in ASSIGN_RE.finditer(masked):
        expr_start = match.end()
        expr = stripped[expr_start:expression_end(stripped, expr_start)].strip()
        if expr:
            assignments.append((match.group("name"), expr))

    changed = True
    while changed:
        changed = False
        for name, expr in assignments:
            kinds = exec_function_kinds(expr, argv_exec_vars, sync_exec_vars)
            if "argv" in kinds and name not in argv_exec_vars:
                argv_exec_vars.add(name)
                changed = True
            if "sync" in kinds and name not in sync_exec_vars:
                sync_exec_vars.add(name)
                changed = True
    return argv_exec_vars, sync_exec_vars


def add_argv_site_if_engine(
    current: set[str],
    relpath: str,
    line: int,
    argv_expr: str,
    engine_vars: set[str],
    argv_vars: set[str],
) -> bool:
    if argv_expr_uses_engine_head(argv_expr, engine_vars, argv_vars):
        current.add(f"{relpath}:line={line}:argv:engine-binary")
        return True
    return False


def add_sync_site_if_engine(
    current: set[str],
    relpath: str,
    line: int,
    cmd_expr: str,
    engine_vars: set[str],
) -> bool:
    if sync_expr_uses_engine_head(cmd_expr, engine_vars):
        current.add(f"{relpath}:line={line}:sync:engine-binary")
        return True
    return False


def exec_call_sites(
    relpath: str,
    source: str,
    strip_lua_comments_and_strings: Callable[[str], str],
    engine_vars: set[str],
    argv_vars: set[str],
    argv_exec_vars: set[str],
    sync_exec_vars: set[str],
) -> set[str]:
    current: set[str] = set()
    stripped = strip_lua_comments(source)
    masked = strip_lua_comments_and_strings(stripped)
    for match in EXEC_CALL_RE.finditer(masked):
        name = match.group("name")
        call_start = match.end() - 1
        call_end = find_matching(stripped, call_start, "(", ")")
        call_args = stripped[call_start + 1 : call_end - 1]
        line = source.count("\n", 0, match.start()) + 1
        if name in ARGV_EXEC_NAMES:
            argv_expr = argv_expr_from_call_args(call_args)
            if argv_expr is not None:
                add_argv_site_if_engine(current, relpath, line, argv_expr, engine_vars, argv_vars)
        elif name in SYNC_EXEC_NAMES:
            cmd_expr = sync_expr_from_call_args(call_args)
            if cmd_expr is not None:
                add_sync_site_if_engine(current, relpath, line, cmd_expr, engine_vars)
    for match in ANY_CALL_RE.finditer(masked):
        name = match.group("name")
        call_start = match.end() - 1
        call_end = find_matching(stripped, call_start, "(", ")")
        call_args = stripped[call_start + 1 : call_end - 1]
        line = source.count("\n", 0, match.start()) + 1
        if name in {"pcall", "xpcall"}:
            kinds = exec_function_kinds(first_call_arg(call_args), argv_exec_vars, sync_exec_vars)
            detected = False
            if "argv" in kinds:
                for index in (1, 2):
                    argv_expr = argv_expr_from_exec_arg(call_arg(call_args, index))
                    if argv_expr is not None and add_argv_site_if_engine(current, relpath, line, argv_expr, engine_vars, argv_vars):
                        detected = True
                        break
            if not detected and "sync" in kinds:
                for index in (1, 2):
                    cmd_expr = sync_expr_from_exec_arg(call_arg(call_args, index))
                    if cmd_expr is not None and add_sync_site_if_engine(current, relpath, line, cmd_expr, engine_vars):
                        break
            continue

        if name in argv_exec_vars:
            argv_expr = argv_expr_from_call_args(call_args)
            if argv_expr is not None and add_argv_site_if_engine(current, relpath, line, argv_expr, engine_vars, argv_vars):
                continue
        if name in sync_exec_vars:
            cmd_expr = sync_expr_from_call_args(call_args)
            if cmd_expr is not None:
                add_sync_site_if_engine(current, relpath, line, cmd_expr, engine_vars)
            continue

        argv_expr = call_option_field(call_args, "argv")
        if argv_expr is not None:
            add_argv_site_if_engine(current, relpath, line, argv_expr, engine_vars, argv_vars)
        cmd_expr = call_option_field(call_args, "cmd")
        if cmd_expr is not None:
            add_sync_site_if_engine(current, relpath, line, cmd_expr, engine_vars)
    return current


def load_allowlist(path: Path) -> set[str]:
    if not path.exists():
        return set()
    return {line.strip() for line in path.read_text(encoding="utf-8").splitlines() if line.strip() and not line.lstrip().startswith("#")}


def source_sites(relpath: str, source: str, strip_lua_comments_and_strings: Callable[[str], str], lua_string_literals: Callable[[str], Iterable]) -> set[str]:
    del lua_string_literals
    engine_vars, argv_vars = collect_engine_bindings(source, strip_lua_comments_and_strings)
    argv_exec_vars, sync_exec_vars = collect_exec_function_bindings(source, strip_lua_comments_and_strings)
    return exec_call_sites(
        relpath,
        source,
        strip_lua_comments_and_strings,
        engine_vars,
        argv_vars,
        argv_exec_vars,
        sync_exec_vars,
    )


def sites(root: Path, package_roots: list[Path], read_text, rel, strip_lua_comments_and_strings, lua_string_literals) -> set[str]:
    current: set[str] = set()
    for packages in package_roots:
        for package in sorted(packages.glob("*")):
            if not package.is_dir():
                continue
            for path in sorted(package.rglob("*.lua")):
                if path.is_file():
                    current.update(source_sites(rel(root, path), read_text(path), strip_lua_comments_and_strings, lua_string_literals))
    return current


def ratchet_messages(current: set[str], allowlist: set[str]) -> list[str]:
    messages = [
        f"{site} shells out to the framework binary; use an in-process SDK primitive instead or list pre-existing debt in {ALLOWLIST}"
        for site in sorted(current - allowlist)
    ]
    messages.extend(
        f"{site} listed in {ALLOWLIST} but no longer detected; prune the stale entry"
        for site in sorted(allowlist - current)
    )
    return messages
