"""Ratchet for the dogfood operator / host-run launch boundary."""

from __future__ import annotations

import re
from dataclasses import dataclass
from pathlib import Path


DOGFOOD_SCRIPT = ".claude/skills/dogfood-github-devloop/dogfood.sh"
LAUNCH_PATH_FUNCTIONS = ("launch_one", "start_one", "restart_one")
FUNCTION_RE = re.compile(r"^(?P<name>[A-Za-z_][A-Za-z0-9_]*)\s*\(\)\s*\{")
RUN_SH_SUPERVISE_RE = re.compile(r"scripts/run\.sh[\"']?\s+supervise\b")
PACKAGE_ROOT_RE = re.compile(r"(?<![A-Za-z0-9_-])--package-root(?![A-Za-z0-9_-])")
RUNTIME_ROOT_SET_RE = re.compile(
    r"(^|[\s;&|(\[])"
    r"(?:export\s+|env\s+)?FKST_RUNTIME_ROOT\s*=",
    re.MULTILINE,
)
DIRECT_BIN_SUPERVISE_RE = re.compile(
    r"(^|[\s;&|(\[])"
    r"(?:nohup\s+|exec\s+|command\s+)?"
    r"(?:(?:[\"'])?\$\{?BIN\}?(?:[\"'])?|[^\s;&|()]*fkst-framework(?:[\"'])?)"
    r"\s+supervise\b",
    re.MULTILINE,
)


@dataclass(frozen=True)
class FunctionBlock:
    name: str
    start_line: int
    source: str


def strip_shell_comments(source: str) -> str:
    lines: list[str] = []
    for line in source.splitlines():
        lines.append(line[: shell_comment_index(line)])
    return "\n".join(lines)


def shell_comment_index(line: str) -> int:
    quote: str | None = None
    escaped = False
    for index, char in enumerate(line):
        if escaped:
            escaped = False
            continue
        if char == "\\" and quote != "'":
            escaped = True
            continue
        if quote is not None:
            if char == quote:
                quote = None
            continue
        if char in ("'", '"'):
            quote = char
            continue
        if char == "#":
            return index
    return len(line)


def shell_functions(source: str) -> dict[str, FunctionBlock]:
    lines = strip_shell_comments(source).splitlines()
    functions: dict[str, FunctionBlock] = {}
    index = 0
    while index < len(lines):
        match = FUNCTION_RE.match(lines[index])
        if match is None:
            index += 1
            continue
        name = match.group("name")
        start = index
        end = index + 1
        while end < len(lines) and lines[end] != "}":
            end += 1
        functions[name] = FunctionBlock(
            name=name,
            start_line=start + 1,
            source="\n".join(lines[start : min(end + 1, len(lines))]),
        )
        index = end + 1
    return functions


def line_for_source_match(source: str, match: re.Match[str]) -> int:
    return source.count("\n", 0, match.start()) + 1


def launch_path_source(functions: dict[str, FunctionBlock]) -> str:
    return "\n".join(functions[name].source for name in LAUNCH_PATH_FUNCTIONS if name in functions)


def repository_messages(root: Path) -> list[str]:
    path = root / DOGFOOD_SCRIPT
    if not path.exists():
        return []
    source = path.read_text(encoding="utf-8")
    stripped_source = strip_shell_comments(source)
    functions = shell_functions(source)
    messages: list[str] = []
    missing = [name for name in LAUNCH_PATH_FUNCTIONS if name not in functions]
    for name in missing:
        messages.append(f"{DOGFOOD_SCRIPT} is missing launch-path function {name}()")
    if missing:
        return messages

    combined = launch_path_source(functions)
    if RUN_SH_SUPERVISE_RE.search(combined) is None:
        messages.append(
            f"{DOGFOOD_SCRIPT} launch path must delegate through scripts/run.sh supervise; dogfood must not own host launch"
        )

    checks = (
        (PACKAGE_ROOT_RE, "constructs --package-root; package-root wiring belongs to scripts/run.sh supervise"),
        (RUNTIME_ROOT_SET_RE, "sets FKST_RUNTIME_ROOT; runtime scratch ownership belongs to scripts/run.sh supervise"),
        (DIRECT_BIN_SUPERVISE_RE, "invokes the framework BIN directly for supervise; launch must route through scripts/run.sh supervise"),
    )
    for pattern, message in checks:
        for match in pattern.finditer(stripped_source):
            messages.append(f"{DOGFOOD_SCRIPT}:{line_for_source_match(stripped_source, match)} {message}")
    return messages


def check(root: Path, violations: list[str], add) -> None:
    if not (root / DOGFOOD_SCRIPT).exists():
        return
    for message in repository_messages(root):
        add(violations, "G-DOGFOOD-BOUNDARY", message)
