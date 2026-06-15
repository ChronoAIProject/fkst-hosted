"""gh/git Ports&Adapters migration ratchet."""

from __future__ import annotations

import re
import shlex
from dataclasses import dataclass
from pathlib import Path


ALLOWLIST = "migration/gh-git-adapter.allowlist"
ENTRY_FILES = {"std/github.lua", "std/git.lua", "std/github_fake.lua", "std/git_fake.lua"}
DIR_PREFIXES = ("std/github/", "std/git/")
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


def is_adapter_path(relpath: str) -> bool:
    if relpath in ENTRY_FILES or relpath.startswith(DIR_PREFIXES):
        return True
    parts = relpath.split("/")
    if len(parts) < 4 or parts[0] != "packages" or parts[2] != "std":
        return False
    std_relpath = "/".join(parts[2:])
    return std_relpath in ENTRY_FILES or std_relpath.startswith(DIR_PREFIXES)


def sources(root: Path, packages: Path, read_text, rel) -> dict[str, str]:
    paths: list[Path] = []
    if packages.exists():
        paths.extend(path for path in sorted(packages.rglob("*.lua")) if path.is_file())
    std = root / "std"
    if std.exists():
        paths.extend(path for path in sorted(std.rglob("*.lua")) if path.is_file())
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


def command_head(command: str | None) -> str | None:
    if command is None:
        return None
    words = split_shell_words(normalize_shell_prefix(command))
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


def normalized_command_head(command: str | None) -> str | None:
    return command_head(command)


def nearest_call_name(mask: str, literal_start: int) -> str | None:
    cursor = literal_start - 1
    while cursor >= 0 and mask[cursor].isspace():
        cursor -= 1
    depth = 0
    while cursor >= 0:
        char = mask[cursor]
        if char == ")":
            depth += 1
        elif char == "(":
            if depth == 0:
                prefix = mask[:cursor].rstrip()
                match = CALL_NAME_RE.search(prefix)
                if match is None:
                    return None
                return re.sub(r"\s+", "", match.group("name"))
            depth -= 1
        elif depth == 0 and char in "\n;{}":
            return None
        cursor -= 1
    return None


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


def is_message_literal(mask: str, literal_start: int) -> bool:
    return is_message_call_name(nearest_call_name(mask, literal_start))


def command_heads(source: str) -> set[str]:
    mask = lua_code_mask(source)
    literals = lua_string_literals(source)
    literals_by_start = {literal.start: literal for literal in literals}
    heads: set[str] = set()
    previous_literals: list[LuaStringLiteral] = []
    for literal in literals:
        if prior_literal_in_concat(source, literal, previous_literals):
            previous_literals.append(literal)
            continue
        head = normalized_command_head(
            command_prefix_for_literal(source, mask, literal, literals_by_start)
        )
        if head is not None:
            if not is_message_literal(mask, literal.start):
                heads.add(head)
        previous_literals.append(literal)
    return heads


def command_heads_by_file(sources: dict[str, str]) -> dict[str, set[str]]:
    current: dict[str, set[str]] = {}
    for path, source in sorted(sources.items()):
        if not path.endswith(".lua") or "/tests/" in path or is_adapter_path(path):
            continue
        if not (path.startswith("packages/") or path.startswith("std/")):
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
                f"{path} constructs a new gh/git command head '{head}' not in the allowlist baseline; migrate it to std.github/std.git"
            )
        for head in sorted(allowlisted_heads - current_heads):
            messages.append(
                f"{path} no longer constructs '{head}'; update its entry in {ALLOWLIST} (it must shrink)"
            )
    return messages
