"""Structural ratchet for identity-carrying codex dispatch.

The canonical path is ``workflow.codex.dispatch(identity, opts)``. Raw
``spawn_codex`` and ``spawn_codex_sync`` may exist for non-identity work, but if
their opts carry ``role``, ``proposal_id``, and ``dedup_key`` they must be inside
that wrapper.
"""

from __future__ import annotations

import re
from dataclasses import dataclass
from pathlib import Path

import ratchet_base


ALLOWLIST = "migration/live-run-dispatch.allowlist"
IDENTITY_FIELDS = ("role", "proposal_id", "dedup_key")
SPAWN_RE = re.compile(r"\b(?P<callee>spawn_codex(?:_sync)?)\s*\(")
VAR_CALL_RE = re.compile(r"^\s*(?P<var>[A-Za-z_][A-Za-z0-9_]*)\s*$")
ASSIGN_RE_TEMPLATE = r"\b{var}\s*(?:\.|\[\s*['\"]){field}(?:\s*['\"]\s*\])?\s*="
TABLE_ASSIGN_RE_TEMPLATE = r"\blocal\s+{var}\s*=\s*\{{(?P<body>.*?)\}}"


@dataclass(frozen=True)
class LiveRunDispatchSite:
    path: str
    line: int
    callee: str

    def allowlist_key(self) -> str:
        return f"{self.path}:{self.line}"


def parse_allowlist_lines(lines: list[str]) -> set[str]:
    entries: set[str] = set()
    for raw in lines:
        line = raw.split("#", 1)[0].strip()
        if not line:
            continue
        if ":" not in line:
            raise ValueError(f"invalid {ALLOWLIST} line: {raw}")
        path_part, line_part = line.rsplit(":", 1)
        if not (path_part.startswith("packages/") or path_part.startswith("libraries/")):
            raise ValueError(f"invalid {ALLOWLIST} line: {raw}")
        if not path_part.endswith(".lua") or not line_part.isdigit() or int(line_part) < 1:
            raise ValueError(f"invalid {ALLOWLIST} line: {raw}")
        entries.add(line)
    return entries


def load_allowlist(path: Path) -> set[str]:
    if not path.exists():
        return set()
    return parse_allowlist_lines(path.read_text(encoding="utf-8").splitlines())


def allowlist_at_dev_base(root: Path) -> tuple[str, set[str] | None]:
    try:
        status, shown = ratchet_base.file_at_base(root, ALLOWLIST)
        if status != "present":
            return status, None
        assert shown is not None
        return "present", parse_allowlist_lines(shown.splitlines())
    except Exception:
        return "unresolved", None


def _mask_span(chars: list[str], start: int, end: int) -> None:
    for index in range(start, min(end, len(chars))):
        if chars[index] != "\n":
            chars[index] = " "


def _long_bracket_at(text: str, index: int) -> tuple[int, str] | None:
    if index >= len(text) or text[index] != "[":
        return None
    cursor = index + 1
    while cursor < len(text) and text[cursor] == "=":
        cursor += 1
    if cursor >= len(text) or text[cursor] != "[":
        return None
    return cursor - index + 1, "]" + ("=" * (cursor - index - 1)) + "]"


def lua_code_mask(source: str) -> str:
    chars = list(source)
    cursor = 0
    while cursor < len(source):
        if source.startswith("--", cursor):
            long = _long_bracket_at(source, cursor + 2)
            if long is not None:
                opener_len, closer = long
                body_start = cursor + 2 + opener_len
                close = source.find(closer, body_start)
                end = len(source) if close == -1 else close + len(closer)
                _mask_span(chars, cursor, end)
                cursor = end
                continue
            end = source.find("\n", cursor)
            end = len(source) if end == -1 else end
            _mask_span(chars, cursor, end)
            cursor = end
            continue
        long = _long_bracket_at(source, cursor)
        if long is not None:
            opener_len, closer = long
            body_start = cursor + opener_len
            close = source.find(closer, body_start)
            end = len(source) if close == -1 else close + len(closer)
            _mask_span(chars, cursor, end)
            cursor = end
            continue
        if source[cursor] in ("'", '"'):
            quote = source[cursor]
            end = cursor + 1
            while end < len(source):
                if source[end] == "\\":
                    end += 2
                    continue
                if source[end] == quote:
                    end += 1
                    break
                end += 1
            _mask_span(chars, cursor, end)
            cursor = end
            continue
        cursor += 1
    return "".join(chars)


def source_line_number(source: str, index: int) -> int:
    return source[: max(0, index)].count("\n") + 1


def matching_paren(masked: str, open_index: int) -> int | None:
    depth = 0
    cursor = open_index
    while cursor < len(masked):
        char = masked[cursor]
        if char == "(":
            depth += 1
        elif char == ")":
            depth -= 1
            if depth == 0:
                return cursor
        cursor += 1
    return None


def function_ranges(masked: str) -> list[tuple[int, int, str]]:
    declarations: list[tuple[int, str]] = []
    offset = 0
    for line in masked.splitlines(keepends=True):
        match = re.match(r"\s*(?:local\s+)?function\s+([A-Za-z_][A-Za-z0-9_.:]*)\s*\(", line)
        if match:
            declarations.append((offset, match.group(1)))
        offset += len(line)
    ranges: list[tuple[int, int, str]] = []
    for index, (start, name) in enumerate(declarations):
        end = declarations[index + 1][0] if index + 1 < len(declarations) else len(masked)
        ranges.append((start, end, name))
    return ranges


def in_workflow_dispatch(path: str, masked: str, index: int) -> bool:
    if path != "libraries/workflow/codex.lua":
        return False
    for start, end, name in function_ranges(masked):
        if start <= index < end and name == "M.dispatch":
            return True
    return False


def call_argument(masked: str, match: re.Match[str]) -> tuple[str, int] | None:
    open_index = match.end() - 1
    close_index = matching_paren(masked, open_index)
    if close_index is None:
        return None
    return masked[open_index + 1 : close_index], close_index


def table_carries_identity(text: str) -> bool:
    return all(re.search(rf"\b{field}\s*=", text) for field in IDENTITY_FIELDS)


def variable_has_identity(masked: str, var: str, before_index: int) -> bool:
    before = masked[:before_index]
    has_fields = all(
        re.search(ASSIGN_RE_TEMPLATE.format(var=re.escape(var), field=field), before)
        for field in IDENTITY_FIELDS
    )
    if has_fields:
        return True
    for match in re.finditer(TABLE_ASSIGN_RE_TEMPLATE.format(var=re.escape(var)), before, re.DOTALL):
        if table_carries_identity(match.group("body")):
            return True
    return False


def raw_spawn_carries_identity(masked: str, match: re.Match[str]) -> bool:
    parsed = call_argument(masked, match)
    if parsed is None:
        return False
    args, _ = parsed
    if table_carries_identity(args):
        return True
    var_match = VAR_CALL_RE.match(args)
    return bool(var_match and variable_has_identity(masked, var_match.group("var"), match.start()))


def current_violations(sources: dict[str, str]) -> list[LiveRunDispatchSite]:
    violations: list[LiveRunDispatchSite] = []
    for path in sorted(sources):
        if not (path.startswith("packages/") or path.startswith("libraries/")):
            continue
        source = sources[path]
        masked = lua_code_mask(source)
        for match in SPAWN_RE.finditer(masked):
            if in_workflow_dispatch(path, masked, match.start()):
                continue
            if raw_spawn_carries_identity(masked, match):
                violations.append(LiveRunDispatchSite(path, source_line_number(masked, match.start()), match.group("callee")))
    return violations


def source_roots(root: Path) -> list[Path]:
    return [path for path in (root / "packages", root / "libraries") if path.exists()]


def repository_sources(root: Path) -> dict[str, str]:
    sources: dict[str, str] = {}
    for source_root in source_roots(root):
        prefix = source_root.name
        for path in sorted(source_root.rglob("*.lua")):
            rel = f"{prefix}/{path.relative_to(source_root).as_posix()}"
            sources[rel] = path.read_text(encoding="utf-8")
    return sources


def ratchet_messages(
    current: list[LiveRunDispatchSite] | set[str],
    allowlist: set[str],
    base_allowlist: set[str] | None = None,
) -> list[str]:
    if isinstance(current, set):
        if base_allowlist is None:
            return []
        return [
            f"{site} grows {ALLOWLIST} relative to dev; migrate the dispatch to workflow.codex.dispatch instead"
            for site in sorted(current - base_allowlist)
        ]
    current_keys = {site.allowlist_key() for site in current}
    messages = [
        f"{site.allowlist_key()} raw identity-carrying {site.callee} is forbidden outside workflow.codex.dispatch"
        for site in current
        if site.allowlist_key() not in allowlist
    ]
    stale_candidates = allowlist - current_keys
    if base_allowlist is not None:
        stale_candidates = stale_candidates & base_allowlist
    messages.extend(
        f"{site} no longer matches a raw identity-carrying codex dispatch; prune {ALLOWLIST}"
        for site in sorted(stale_candidates)
    )
    if base_allowlist is not None:
        messages.extend(
            f"{site} grows {ALLOWLIST} relative to dev; migrate the dispatch to workflow.codex.dispatch instead"
            for site in sorted(allowlist - base_allowlist)
        )
    return messages


def repository_messages(root: Path, allowlist_dir: Path | None = None, enforce_base: bool = True) -> list[str]:
    allow_path = root / ALLOWLIST if allowlist_dir is None else allowlist_dir / Path(ALLOWLIST).name
    allowlist = load_allowlist(allow_path)
    base_status, base_allowlist = allowlist_at_dev_base(root) if enforce_base else ("absent", None)
    messages: list[str] = []
    if base_status == "unresolved":
        messages.append("cannot resolve dev base allowlist to enforce shrink-only live-run-dispatch ratchet; ensure CI provides the dev ref")
    messages.extend(ratchet_messages(current_violations(repository_sources(root)), allowlist, base_allowlist))
    return messages
