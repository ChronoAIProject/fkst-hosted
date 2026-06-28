"""gh/git Ports&Adapters migration ratchet."""

from __future__ import annotations

import re
import shlex
from dataclasses import dataclass
from pathlib import Path


ALLOWLIST = "migration/gh-git-adapter.allowlist"
ENTRY_FILES = {"libraries/forge/github.lua", "libraries/forge/git.lua", "libraries/forge/github_fake.lua", "libraries/forge/git_fake.lua"}
DIR_PREFIXES = ("libraries/forge/github/", "libraries/forge/git/")
ENV_ASSIGN_RE = re.compile(r"^(?:[A-Za-z_][A-Za-z0-9_]*=\S+\s+)+")
CD_RE = re.compile(r"^cd\s+(?:'[^']*'|\"[^\"]*\"|[^\s;&]+)\s*(?:&&|;)\s*")
SHELL_C_RE = re.compile(
    r"^(?:(?:/[^/\s]+)*/)?(?:sh|bash)\s+-c\s+(?P<quote>['\"])(?P<body>.*)(?P=quote)\s*$",
    re.DOTALL,
)
CALL_NAME_RE = re.compile(
    r"(?P<name>(?:[A-Za-z_][A-Za-z0-9_]*\s*[\.:]\s*)*[A-Za-z_][A-Za-z0-9_]*)\s*$"
)
MESSAGE_CALLS = {"log", "raise", "error", "assert", "print"}
MESSAGE_METHODS = {"info", "warn", "warning", "debug", "error"}
EXEC_WRAPPER_NAMES = {"gh_exec", "run_cmd", "run_exec", "run_gh", "run_git", "run_gh_cmd"}
OPTION_ARG_FLAGS = {
    "-C",
    "-c",
    "-R",
    "--cwd",
    "--git-dir",
    "--hostname",
    "--namespace",
    "--repo",
    "--work-tree",
}
PLACEHOLDER = "__FKST_DYNAMIC__"


@dataclass(frozen=True)
class LuaStringLiteral:
    start: int
    end: int
    content: str


@dataclass(frozen=True)
class CallContext:
    name: str | None
    open_paren: int
    start: int
    end: int
    arg_starts: tuple[int, ...]


def is_adapter_path(relpath: str) -> bool:
    return relpath in ENTRY_FILES or relpath.startswith(DIR_PREFIXES)


def sources(root: Path, packages: Path, read_text, rel) -> dict[str, str]:
    paths: list[Path] = []
    if packages.exists():
        paths.extend(path for path in sorted(packages.rglob("*.lua")) if path.is_file())
    forge = root / "libraries" / "forge"
    if forge.exists():
        paths.extend(path for path in sorted(forge.rglob("*.lua")) if path.is_file())
    return {rel(root, path): read_text(path) for path in paths}


def load_allowlist(path: Path) -> dict[str, set[str]]:
    if not path.exists():
        return {}
    entries: dict[str, set[str]] = {}
    current_path: str | None = None
    for raw in path.read_text(encoding="utf-8").splitlines():
        if not raw.strip() or raw.lstrip().startswith("#"):
            continue
        if not raw.startswith((" ", "\t")) and raw.rstrip().endswith(":"):
            current_path = raw.strip()[:-1]
            entries.setdefault(current_path, set())
            continue
        stripped = raw.strip()
        if current_path is None or not stripped.startswith("- "):
            raise ValueError(f"invalid {ALLOWLIST} line: {raw}")
        entries[current_path].add(stripped[2:].strip())
    return entries


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
    for index in range(start, end):
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


def parse_literal(text: str, cursor: int) -> tuple[str, int] | None:
    if cursor >= len(text):
        return None
    if text[cursor] in ("'", '"'):
        end = end_of_quoted_string(text, cursor)
        content_end = end - 1 if end <= len(text) and text[end - 1] == text[cursor] else end
        return text[cursor + 1 : content_end], end
    bracket = long_bracket_at(text, cursor)
    if bracket is None:
        return None
    opener_len, closer = bracket
    body_start = cursor + opener_len
    close_start = text.find(closer, body_start)
    body_end = len(text) if close_start == -1 else close_start
    end = len(text) if close_start == -1 else close_start + len(closer)
    return text[body_start:body_end], end


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
            literals.append(LuaStringLiteral(start, end, text[cursor + 1 : content_end]))
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
            literals.append(LuaStringLiteral(start, end, text[body_start:body_end]))
            cursor = end
            continue
        cursor += 1
    return literals


def skip_expression_prefix(text: str, cursor: int) -> int:
    while cursor < len(text) and text[cursor].isspace():
        cursor += 1
    while cursor < len(text) and text[cursor] == "(":
        cursor += 1
        while cursor < len(text) and text[cursor].isspace():
            cursor += 1
    return cursor


def skip_whitespace(text: str, cursor: int) -> int:
    while cursor < len(text) and text[cursor].isspace():
        cursor += 1
    return cursor


def skip_dynamic_operand(mask: str, cursor: int) -> int:
    cursor = skip_whitespace(mask, cursor)
    depth = 0
    while cursor < len(mask):
        if depth == 0 and mask.startswith("..", cursor):
            return cursor
        char = mask[cursor]
        if char in "([{":
            depth += 1
        elif char in ")]}" and depth > 0:
            depth -= 1
        elif depth == 0 and char in ",;":
            return cursor
        cursor += 1
    return cursor


def command_prefix_for_literal(
    text: str,
    mask: str,
    literal: LuaStringLiteral,
    literals_by_start: dict[int, LuaStringLiteral],
) -> str:
    parts = [literal.content]
    cursor = literal.end
    while True:
        cursor = skip_whitespace(text, cursor)
        if not text.startswith("..", cursor):
            break
        cursor = skip_expression_prefix(text, cursor + 2)
        next_literal = literals_by_start.get(cursor)
        if next_literal is not None:
            parts.append(next_literal.content)
            cursor = next_literal.end
            continue
        parts.append(" " + PLACEHOLDER + " ")
        cursor = skip_dynamic_operand(mask, cursor)
    return "".join(parts)


def prior_literal_in_concat(
    text: str,
    literal: LuaStringLiteral,
    previous_literals: list[LuaStringLiteral],
) -> bool:
    cursor = literal.start - 1
    while cursor >= 0 and text[cursor].isspace():
        cursor -= 1
    if cursor < 1 or text[cursor - 1 : cursor + 1] != "..":
        return False
    boundary = max(
        text.rfind("=", 0, cursor - 1),
        text.rfind(",", 0, cursor - 1),
        text.rfind(";", 0, cursor - 1),
        text.rfind("{", 0, cursor - 1),
        text.rfind("(", 0, cursor - 1),
    )
    return any(previous.end > boundary for previous in previous_literals)


def normalize_shell_prefix(command: str) -> str:
    command = lua_unescape_for_shell(command).lstrip()
    for _ in range(6):
        before = command
        command = ENV_ASSIGN_RE.sub("", command.lstrip())
        command = CD_RE.sub("", command.lstrip())
        shell = SHELL_C_RE.match(command.lstrip())
        if shell is not None:
            command = shell.group("body")
        if command == before:
            break
    return command.lstrip()


def lua_unescape_for_shell(command: str) -> str:
    return (
        command.replace(r"\"", '"')
        .replace(r"\'", "'")
        .replace(r"\\", "\\")
    )


def split_shell_words(command: str) -> list[str]:
    try:
        return shlex.split(command, comments=False, posix=True)
    except ValueError:
        return command.split()


def skip_leading_options(words: list[str], index: int) -> int:
    while index < len(words) and words[index].startswith("-"):
        option = words[index]
        index += 1
        if option in OPTION_ARG_FLAGS and index < len(words):
            index += 1
    return index


def command_head_for_words(words: list[str]) -> str | None:
    if not words:
        return None
    tool = words[0].rsplit("/", 1)[-1]
    if tool not in {"gh", "git"}:
        return None
    index = skip_leading_options(words, 1)
    if index >= len(words):
        return tool
    first = words[index]
    if first == PLACEHOLDER:
        return tool
    if first.startswith("-"):
        return tool
    return " ".join([tool, first])


def command_head(command: str | None) -> str | None:
    if command is None:
        return None
    return command_head_for_words(split_shell_words(normalize_shell_prefix(command)))


def normalized_command_head(command: str | None) -> str | None:
    return command_head(command)


def call_name_before(mask: str, open_paren: int) -> str | None:
    cursor = open_paren - 1
    while cursor >= 0 and mask[cursor].isspace():
        cursor -= 1
    end = cursor + 1
    while cursor >= 0 and (mask[cursor].isalnum() or mask[cursor] in "_.:"):
        cursor -= 1
    name = mask[cursor + 1 : end]
    if not name or name[0].isdigit():
        return None
    parts = name.replace(":", ".").split(".")
    if any(part == "" or part[0].isdigit() for part in parts):
        return None
    return name


def lua_call_contexts(mask: str) -> list[CallContext]:
    contexts: list[CallContext] = []
    stack: list[dict[str, int | str | list[int] | None]] = []
    cursor = 0
    while cursor < len(mask):
        char = mask[cursor]
        if char == "(":
            stack.append({
                "kind": char,
                "name": call_name_before(mask, cursor),
                "open": cursor,
                "arg_starts": [cursor + 1],
            })
        elif char in "[{":
            stack.append({"kind": char, "name": None, "open": cursor, "arg_starts": []})
        elif char in ")]}":
            if stack:
                entry = stack.pop()
                if entry["kind"] == "(":
                    contexts.append(CallContext(
                        name=None if entry["name"] is None else str(entry["name"]),
                        open_paren=int(entry["open"]),
                        start=int(entry["open"]) + 1,
                        end=cursor,
                        arg_starts=tuple(int(start) for start in entry["arg_starts"]),
                    ))
        elif char == "," and stack:
            entry = stack[-1]
            if entry["kind"] == "(":
                arg_starts = entry["arg_starts"]
                assert isinstance(arg_starts, list)
                arg_starts.append(cursor + 1)
        cursor += 1
    return contexts


def nearest_call(contexts: list[CallContext], literal_start: int) -> CallContext | None:
    best: CallContext | None = None
    for context in contexts:
        if context.name is not None and context.start <= literal_start <= context.end:
            if best is None or context.start > best.start:
                best = context
    return best


def nearest_call_name(contexts: list[CallContext], literal_start: int) -> str | None:
    call = nearest_call(contexts, literal_start)
    return None if call is None else call.name


def is_message_call_name(name: str | None) -> bool:
    if name is None:
        return False
    parts = name.replace(":", ".").split(".")
    if len(parts) == 1:
        return parts[0] in MESSAGE_CALLS
    method = parts[-1]
    receiver = ".".join(parts[:-1])
    if method in MESSAGE_METHODS:
        return True
    return receiver == "core" and method.startswith("log")


def is_message_literal(contexts: list[CallContext], literal_start: int) -> bool:
    return is_message_call_name(nearest_call_name(contexts, literal_start))


def is_exec_wrapper_call_name(name: str | None) -> bool:
    if name is None:
        return False
    parts = name.replace(":", ".").split(".")
    return parts[-1] in EXEC_WRAPPER_NAMES


def argument_index_at(context: CallContext, cursor: int) -> int:
    index = 0
    for start in context.arg_starts:
        if start <= cursor:
            index += 1
    return max(0, index - 1)


def is_exec_wrapper_context_literal(contexts: list[CallContext], literal_start: int) -> bool:
    call = nearest_call(contexts, literal_start)
    if call is None:
        return False
    return is_exec_wrapper_call_name(call.name) and argument_index_at(call, literal_start) > 0


def is_exec_argv_call_name(name: str | None) -> bool:
    if name is None:
        return False
    parts = name.replace(":", ".").split(".")
    return parts[-1] == "exec_argv"


def is_excluded_literal(contexts: list[CallContext], literal_start: int) -> bool:
    return (
        is_message_literal(contexts, literal_start)
        or is_exec_wrapper_context_literal(contexts, literal_start)
    )


def matching_table_close(mask: str, open_brace: int) -> int | None:
    depth = 0
    cursor = open_brace
    while cursor < len(mask):
        char = mask[cursor]
        if char == "{":
            depth += 1
        elif char == "}":
            depth -= 1
            if depth == 0:
                return cursor
        cursor += 1
    return None


def argv_table_bounds_for_literal(mask: str, literal_start: int) -> tuple[int, int] | None:
    prefix = mask[:literal_start]
    match = re.search(r"argv\s*=\s*\{\s*$", prefix)
    if match is None:
        return None
    open_brace = prefix.rfind("{", match.start(), match.end())
    if open_brace < 0:
        return None
    close_brace = matching_table_close(mask, open_brace)
    if close_brace is None or close_brace <= literal_start:
        return None
    return open_brace, close_brace


def skip_table_operand(mask: str, cursor: int, table_end: int) -> int:
    depth = 0
    while cursor < table_end:
        char = mask[cursor]
        if char in "([{":
            depth += 1
        elif char in ")]}" and depth > 0:
            depth -= 1
        elif depth == 0 and char == ",":
            return cursor
        cursor += 1
    return cursor


def argv_words_from_table(
    source: str,
    mask: str,
    open_brace: int,
    close_brace: int,
    literals_by_start: dict[int, LuaStringLiteral],
) -> list[str]:
    words: list[str] = []
    cursor = open_brace + 1
    while cursor < close_brace:
        while cursor < close_brace and source[cursor].isspace():
            cursor += 1
        if cursor >= close_brace:
            break
        literal = literals_by_start.get(cursor)
        if literal is not None:
            words.append(literal.content)
            cursor = literal.end
        else:
            end = skip_table_operand(mask, cursor, close_brace)
            if mask[cursor:end].strip():
                words.append(PLACEHOLDER)
            cursor = end
        while cursor < close_brace and source[cursor].isspace():
            cursor += 1
        if cursor < close_brace and source[cursor] == ",":
            cursor += 1
    return words


def exec_argv_head_for_literal(
    source: str,
    mask: str,
    contexts: list[CallContext],
    literal: LuaStringLiteral,
    literals_by_start: dict[int, LuaStringLiteral],
) -> str | None:
    call = nearest_call(contexts, literal.start)
    if call is None or not is_exec_argv_call_name(call.name):
        return None
    bounds = argv_table_bounds_for_literal(mask, literal.start)
    if bounds is None:
        return None
    open_brace, close_brace = bounds
    words = argv_words_from_table(source, mask, open_brace, close_brace, literals_by_start)
    if not words or words[0] != literal.content:
        return None
    return command_head_for_words(words)


def command_heads(source: str) -> set[str]:
    mask = lua_code_mask(source)
    contexts = lua_call_contexts(mask)
    literals = lua_string_literals(source)
    literals_by_start = {literal.start: literal for literal in literals}
    heads: set[str] = set()
    previous_literals: list[LuaStringLiteral] = []
    for literal in literals:
        if prior_literal_in_concat(source, literal, previous_literals):
            previous_literals.append(literal)
            continue
        head = exec_argv_head_for_literal(source, mask, contexts, literal, literals_by_start)
        if head is None:
            head = normalized_command_head(
                command_prefix_for_literal(source, mask, literal, literals_by_start)
            )
        if head is not None:
            if not is_excluded_literal(contexts, literal.start):
                heads.add(head)
        previous_literals.append(literal)
    return heads


def command_heads_by_file(sources: dict[str, str]) -> dict[str, set[str]]:
    current: dict[str, set[str]] = {}
    for path, source in sorted(sources.items()):
        if not path.endswith(".lua") or "/tests/" in path or is_adapter_path(path):
            continue
        if not (path.startswith("packages/") or path.startswith("libraries/forge/")):
            continue
        heads = command_heads(source)
        if heads:
            current[path] = heads
    return current


def ratchet_messages(sources: dict[str, str], allowlist: dict[str, set[str]], lua_string_literals=None) -> list[str]:
    current = command_heads_by_file(sources)
    messages: list[str] = []
    for path in sorted(set(current) | set(allowlist)):
        current_heads = current.get(path, set())
        allowlisted_heads = allowlist.get(path, set())
        if not allowlisted_heads and path in allowlist:
            messages.append(
                f"{path} has an empty allowlist entry in {ALLOWLIST}; remove the file entry"
            )
        for head in sorted(current_heads - allowlisted_heads):
            messages.append(
                f"{path} constructs a new gh/git command head '{head}' not in the allowlist baseline; migrate it to forge.github/forge.git"
            )
        for head in sorted(allowlisted_heads - current_heads):
            messages.append(
                f"{path} no longer constructs '{head}'; update its entry in {ALLOWLIST} (it must shrink)"
            )
    return messages
