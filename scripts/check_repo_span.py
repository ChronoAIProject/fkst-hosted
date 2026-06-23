#!/usr/bin/env python3
"""G-SPAN guard for github-devloop worker span semantics."""

from __future__ import annotations

import re
from dataclasses import dataclass
from pathlib import Path

START_WORD_RE = re.compile(r"\b(?:start|starts|started|begin|begins|began|beginning)\b", re.IGNORECASE)
COMMENT_STRING_RE = re.compile(r"\bcomment_string\s*\(\s*(?P<quote>[\"'])(?P<key>[A-Za-z0-9_]+)(?P=quote)\s*\)")
STRING_ENTRY_RE = re.compile(r"(?P<key>[A-Za-z0-9_]+)\s*=\s*(?P<quote>[\"'])(?P<value>(?:\\.|(?!\2).)*)(?P=quote)")
LUA_STRING_RE = re.compile(r"(?P<quote>[\"'])(?P<value>(?:\\.|(?!\1).)*)(?P=quote)")
FUNCTION_RE = re.compile(
    r"(?m)^\s*(?:local\s+)?function\s+(?P<name>[A-Za-z_][A-Za-z0-9_]*(?:[.:][A-Za-z_][A-Za-z0-9_]*)?)\s*\((?P<params>[^)]*)\)"
)
END_RE = re.compile(r"(?m)^\s*end\s*$")
WORKER_ROW_RE = re.compile(
    r"from_state\s*=\s*(?P<quote>[\"'])(?P<state>[^\"']+)(?P=quote).*?"
    r"state_kind\s*=\s*(?P<kind_quote>[\"'])worker(?P=kind_quote)",
    re.DOTALL,
)
SPAN_CONTRACT_RE = re.compile(
    r"span_contract\s*=\s*span_contract\s*\(\s*\{(?P<body>.*?)\}\s*\)",
    re.DOTALL,
)
FIELD_STRING_RE = re.compile(r"\b(?P<field>[A-Za-z_][A-Za-z0-9_]*)\s*=\s*(?P<quote>[\"'])(?P<value>[^\"']+)(?P=quote)")
SPAWN_RE = re.compile(r"\bspawn_codex_sync\s*\(")
CALL_RE = re.compile(r"(?<!function\s)\b(?:[A-Za-z_][A-Za-z0-9_]*[.:])?(?P<name>[A-Za-z_][A-Za-z0-9_]*)\s*\(")


@dataclass(frozen=True)
class FunctionBlock:
    name: str
    params: str
    body: str
    start_line: int


@dataclass(frozen=True)
class SpanContract:
    state: str
    department: str
    durable_start_marker: str
    spawn_predecessor: str
    spawn_function: str | None
    path: str
    line: int


def line_number(source: str, index: int) -> int:
    return source.count("\n", 0, index) + 1


def _unescape_lua_string(value: str) -> str:
    return value.replace('\\"', '"').replace("\\'", "'").replace("\\n", "\n")


def comment_strings(sources: dict[str, str]) -> dict[str, str]:
    values: dict[str, str] = {}
    for path, source in sources.items():
        if not path.endswith("core/strings.lua"):
            continue
        for match in STRING_ENTRY_RE.finditer(source):
            values[match.group("key")] = _unescape_lua_string(match.group("value"))
    return values


def function_blocks(source: str) -> list[FunctionBlock]:
    matches = list(FUNCTION_RE.finditer(source))
    blocks: list[FunctionBlock] = []
    for index, match in enumerate(matches):
        next_start = matches[index + 1].start() if index + 1 < len(matches) else len(source)
        blocks.append(
            FunctionBlock(
                name=match.group("name"),
                params=match.group("params"),
                body=source[match.end() : next_start],
                start_line=line_number(source, match.start()),
            )
        )
    return blocks


def _has_head_sha_dependency(block: FunctionBlock) -> bool:
    params = {part.strip() for part in block.params.split(",")}
    return "head_sha" in params or "head_sha" in block.body


def completion_fact_name_messages(sources: dict[str, str]) -> list[str]:
    strings = comment_strings(sources)
    messages: list[str] = []
    for path, source in sorted(sources.items()):
        if not path.endswith(".lua"):
            continue
        for block in function_blocks(source):
            if "comment_request" not in block.name:
                continue
            if not _has_head_sha_dependency(block):
                continue
            for match in COMMENT_STRING_RE.finditer(block.body):
                key = match.group("key")
                text = strings.get(key, key)
                if START_WORD_RE.search(key) is not None or START_WORD_RE.search(text) is not None:
                    messages.append(
                        f"{path}:{block.start_line} {block.name} completion/output comment uses start wording key {key!r} while requiring post-work field head_sha"
                    )
            for match in LUA_STRING_RE.finditer(block.body):
                text = _unescape_lua_string(match.group("value"))
                if START_WORD_RE.search(text) is not None:
                    messages.append(
                        f"{path}:{block.start_line} {block.name} completion/output comment uses start wording literal while requiring post-work field head_sha"
                    )
                    break
    return messages


def span_contracts(transition_sources: dict[str, str]) -> dict[str, SpanContract]:
    contracts: dict[str, SpanContract] = {}
    for path, source in sorted(transition_sources.items()):
        for row in WORKER_ROW_RE.finditer(source):
            state = row.group("state")
            contract_match = SPAN_CONTRACT_RE.search(source, row.start())
            if contract_match is None:
                continue
            fields = {match.group("field"): match.group("value") for match in FIELD_STRING_RE.finditer(contract_match.group("body"))}
            if fields.get("department") and fields.get("durable_start_marker") and fields.get("spawn_predecessor"):
                contracts[state] = SpanContract(
                    state=state,
                    department=fields["department"],
                    durable_start_marker=fields["durable_start_marker"],
                    spawn_predecessor=fields["spawn_predecessor"],
                    spawn_function=fields.get("spawn_function"),
                    path=path,
                    line=line_number(source, contract_match.start()),
                )
    return contracts


def _worker_states(transition_sources: dict[str, str]) -> list[tuple[str, str, int]]:
    rows: list[tuple[str, str, int]] = []
    for path, source in sorted(transition_sources.items()):
        for match in WORKER_ROW_RE.finditer(source):
            rows.append((match.group("state"), path, line_number(source, match.start())))
    return rows


def _department_spawn_sources(department_sources: dict[str, str], department: str) -> dict[str, str]:
    needle = f"/departments/{department}/"
    return {path: source for path, source in department_sources.items() if needle in path}


def _predecessor_call_before(source: str, function_name: str, index: int) -> int:
    pattern = re.compile(
        r"(?<!function\s)\b" + re.escape(function_name) + r"\s*\(",
        re.DOTALL,
    )
    found = -1
    for match in pattern.finditer(source, 0, index):
        found = match.start()
    return found


def _function_contains_spawn(source: str, function_name: str) -> bool:
    for block in function_blocks(source):
        short_name = block.name.split(".")[-1].split(":")[-1]
        if short_name == function_name and SPAWN_RE.search(block.body) is not None:
            return True
    return False


def _short_function_name(name: str) -> str:
    return name.split(".")[-1].split(":")[-1]


def _function_index(sources: dict[str, str]) -> dict[str, list[FunctionBlock]]:
    index: dict[str, list[FunctionBlock]] = {}
    for source in sources.values():
        for block in function_blocks(source):
            index.setdefault(_short_function_name(block.name), []).append(block)
    return index


def _marker_helper_name(durable_start_marker: str) -> str | None:
    family = durable_start_marker.split()[0]
    family = family.split(":")[0]
    if family == "state":
        return None
    return family.replace("-", "_") + "_marker"


def _state_marker_value(durable_start_marker: str) -> str | None:
    prefix = "state:v1 "
    if not durable_start_marker.startswith(prefix):
        return None
    value = durable_start_marker[len(prefix) :].strip()
    return value or None


def _body_mentions_marker(body: str, durable_start_marker: str) -> bool:
    if durable_start_marker in body:
        return True
    helper_name = _marker_helper_name(durable_start_marker)
    if helper_name is not None and re.search(r"\b" + re.escape(helper_name) + r"\s*\(", body) is not None:
        return True
    state = _state_marker_value(durable_start_marker)
    if state is None:
        return False
    quoted_state = r"(?P<quote>[\"'])" + re.escape(state) + r"(?P=quote)"
    if re.search(r"\bstate_marker\s*\(.*?" + quoted_state, body, re.DOTALL) is not None:
        return True
    if re.search(r"\bhas_state_marker\s*\(.*?" + quoted_state, body, re.DOTALL) is not None:
        return True
    if "current_entity_state" not in body:
        return False
    return re.search(r"\b[A-Za-z_][A-Za-z0-9_]*\.state\s*(?:==|~=)\s*" + quoted_state, body) is not None


def _function_binds_marker(functions: dict[str, list[FunctionBlock]], function_name: str, durable_start_marker: str) -> bool:
    pending = [function_name]
    seen: set[str] = set()
    while pending:
        current = pending.pop()
        if current in seen:
            continue
        seen.add(current)
        for block in functions.get(current, []):
            if _body_mentions_marker(block.body, durable_start_marker):
                return True
            for call in CALL_RE.finditer(block.body):
                callee = call.group("name")
                if callee not in seen and callee in functions:
                    pending.append(callee)
    return False


def spawn_start_messages(transition_sources: dict[str, str], department_sources: dict[str, str], support_sources: dict[str, str] | None = None) -> list[str]:
    contracts = span_contracts(transition_sources)
    functions = _function_index(support_sources or department_sources)
    messages: list[str] = []
    for state, path, line in _worker_states(transition_sources):
        contract = contracts.get(state)
        if contract is None:
            messages.append(f"{path}:{line} worker row with spawn_codex_sync must declare span_contract")
            continue
        if contract.department.startswith("external:"):
            continue
        if not _function_binds_marker(functions, contract.spawn_predecessor, contract.durable_start_marker):
            messages.append(
                f"{contract.path}:{contract.line} span start predecessor {contract.spawn_predecessor!r} does not bind durable start marker {contract.durable_start_marker!r}"
            )
        sources = _department_spawn_sources(department_sources, contract.department)
        if not sources:
            messages.append(f"{contract.path}:{contract.line} span_contract department {contract.department!r} has no scanned department source")
            continue
        saw_spawn = False
        for source_path, source in sorted(sources.items()):
            if contract.spawn_function is not None:
                if not _function_contains_spawn(source, contract.spawn_function):
                    continue
                saw_spawn = True
                call_pattern = re.compile(r"(?<!function\s)\b" + re.escape(contract.spawn_function) + r"\s*\(")
                for call in call_pattern.finditer(source):
                    predecessor = _predecessor_call_before(source, contract.spawn_predecessor, call.start())
                    if predecessor < 0:
                        messages.append(
                            f"{source_path}:{line_number(source, call.start())} {contract.spawn_function} call must be preceded by span start predecessor {contract.spawn_predecessor!r} for durable start marker {contract.durable_start_marker!r}"
                        )
                continue
            for spawn in SPAWN_RE.finditer(source):
                saw_spawn = True
                predecessor = _predecessor_call_before(source, contract.spawn_predecessor, spawn.start())
                if predecessor < 0:
                    messages.append(
                        f"{source_path}:{line_number(source, spawn.start())} spawn_codex_sync must be preceded by span start predecessor {contract.spawn_predecessor!r} for durable start marker {contract.durable_start_marker!r}"
                    )
        if not saw_spawn:
            messages.append(f"{contract.path}:{contract.line} span_contract department {contract.department!r} has no spawn_codex_sync call")
    return messages


def sources(root: Path) -> dict[str, str]:
    found: dict[str, str] = {}
    for package in ("github-devloop", "github-devloop-pr"):
        base = root / "packages" / package
        if not base.exists():
            continue
        for path in sorted(base.rglob("*.lua")):
            if "/tests/" in path.as_posix():
                continue
            rel = path.relative_to(root).as_posix()
            found[rel] = path.read_text(encoding="utf-8")
    devloop = root / "libraries" / "devloop"
    if devloop.exists():
        for path in sorted(devloop.rglob("*.lua")):
            rel = path.relative_to(root).as_posix()
            found[rel] = path.read_text(encoding="utf-8")
    return found


def repository_messages(root: Path) -> list[str]:
    source_map = sources(root)
    transition_sources = {
        path: text
        for path, text in source_map.items()
        if "/core/restart/transitions/" in path or "/restart/issue/transitions/" in path
    }
    department_sources = {path: text for path, text in source_map.items() if "/departments/" in path}
    return completion_fact_name_messages(source_map) + spawn_start_messages(transition_sources, department_sources, source_map)
