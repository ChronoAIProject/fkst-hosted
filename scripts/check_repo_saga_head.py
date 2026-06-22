"""G-SAGA-HEAD guard for workflow.saga department spec placement."""

from __future__ import annotations

import re

SAGA_REQUIRE_RE = re.compile(r"""\brequire\s*(?:\(\s*)?["']workflow\.saga["']""")
SAGA_DEPARTMENT_RE = re.compile(r"\.\s*department\s*[({]")
SAGA_DEPARTMENT_CALL_RE = re.compile(r"\bsaga\s*\.\s*department\s*(?P<form>[({])")
SAGA_DEPARTMENT_NAMED_SPEC_RE = re.compile(
    r"\bsaga\s*\.\s*department\s*\(\s*(?P<spec>[A-Za-z_][A-Za-z0-9_]*)\s*,"
)
LOCAL_FUNCTION_RE = re.compile(r"(?m)^\s*local\s+function\b")


def line_number(source: str, index: int) -> int:
    return source.count("\n", 0, index) + 1


def local_table_declaration_line(source: str, name: str) -> int | None:
    match = re.search(r"(?m)^\s*local\s+" + re.escape(name) + r"\s*=\s*\{", source)
    return None if match is None else line_number(source, match.start())


def violations(sources: dict[str, str], strip_lua_comments_and_strings) -> list[str]:
    messages: list[str] = []
    for path, raw_source in sorted(sources.items()):
        source = strip_lua_comments_and_strings(raw_source)
        if SAGA_REQUIRE_RE.search(raw_source) is None or SAGA_DEPARTMENT_RE.search(source) is None:
            continue
        first_function = LOCAL_FUNCTION_RE.search(source)
        first_function_line = None if first_function is None else line_number(source, first_function.start())
        for call in SAGA_DEPARTMENT_CALL_RE.finditer(source):
            call_line = line_number(source, call.start())
            named = SAGA_DEPARTMENT_NAMED_SPEC_RE.match(source, call.start())
            if named is None:
                messages.append(
                    f"{path}:{call_line} saga.department must pass a named spec first argument declared at file head"
                )
                continue
            spec_name = named.group("spec")
            spec_line = local_table_declaration_line(source, spec_name)
            if spec_line is None:
                messages.append(
                    f"{path}:{call_line} saga.department spec {spec_name!r} must be declared as local {spec_name} = {{ at file head"
                )
                continue
            if first_function_line is not None and spec_line >= first_function_line:
                messages.append(
                    f"{path}:{spec_line} saga.department spec {spec_name!r} must be declared before the first local function at line {first_function_line}"
                )
    return messages
