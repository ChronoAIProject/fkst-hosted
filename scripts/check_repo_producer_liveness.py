#!/usr/bin/env python3
"""Producer-liveness fire_raiser trace assertion ratchet."""

from __future__ import annotations

import re
from dataclasses import dataclass
from pathlib import Path

import ratchet_base


ALLOWLIST = "migration/producer-liveness.allowlist"
TRACE_FIELDS = ("consumer_result", "source_payload", "raised", "routed_to")
RAISER_NAME_RE = re.compile(r"\b(?:name|raiser)\s*=\s*(?P<quote>[\"'])(?P<name>[A-Za-z0-9_.-]+)(?P=quote)")
PRODUCES_STRING_RE = re.compile(r"\bproduces\s*=\s*(?P<quote>[\"'])(?P<queue>[A-Za-z0-9_.-]+)(?P=quote)")
PRODUCES_TABLE_RE = re.compile(r"\bproduces\s*=\s*\{(?P<body>.*?)\}", re.DOTALL)
STRING_RE = re.compile(r"(?P<quote>[\"'])(?P<value>[A-Za-z0-9_.-]+)(?P=quote)")
TEST_START_RE = re.compile(
    r"^\s*(?:test_[A-Za-z0-9_]+|\[\s*[\"']test_[A-Za-z0-9_]+[\"']\s*\])\s*=\s*function\b"
    r"|^\s*function\s+(?:[A-Za-z_][A-Za-z0-9_]*[.:])?test_[A-Za-z0-9_]+\s*\("
)
FIRE_RAISER_RE = re.compile(
    r"(?:(?:local\s+)?(?P<var>[A-Za-z_][A-Za-z0-9_]*)\s*=\s*)?"
    r"\bt\s*\.\s*fire_raiser\s*\(\s*"
    r"(?P<quote>[\"'])(?P<raiser>[A-Za-z0-9_.-]+)(?P=quote)\s*\)"
)
FIRE_RAISER_HEAD_RE = re.compile(r"\bt\s*\.\s*fire_raiser\b")
ASSERTION_CALL_RE = re.compile(r"\b(?:t\s*\.\s*(?:eq|is_true|assert)|assert|error|fail)\s*\(")
IF_HEAD_RE = re.compile(r"\bif\b")
IF_THEN_RE = re.compile(r"\bif\b(?P<condition>.*?)\bthen\b", re.DOTALL)
IF_ASSERTION_ACTION_RE = re.compile(r"\b(?:error|fail)\s*\(|\breturn\s+false\b")
FIRE_RAISER_CHILD_PREFIX_RE = re.compile(
    r"\b(?:[A-Za-z_][A-Za-z0-9_]*\s*\.\s*)?fire_raiser_child\s*\(\s*$"
)
LUA_WORD_RE = re.compile(r"\b[A-Za-z_][A-Za-z0-9_]*\b")


@dataclass(frozen=True, order=True)
class ProducerRaiser:
    package: str
    name: str
    path: str
    produces: tuple[str, ...]

    def key(self) -> str:
        return f"{self.package}.{self.name}"

    def label(self) -> str:
        queues = ",".join(self.produces) if self.produces else "<unknown>"
        return f"{self.key()} ({self.path} -> {queues})"


@dataclass(frozen=True, order=True)
class ProducerLivenessContract:
    package: str
    producer_id: str
    trigger_source: str
    runtime_gate: str
    adversarial_fixture: str

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


def mask_lua_comments_and_strings(text: str) -> str:
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


def block_delta(line: str) -> int:
    tokens = LUA_WORD_RE.findall(line)
    delta = 0
    for index, token in enumerate(tokens):
        if token in {"function", "do", "repeat"}:
            delta += 1
        elif token == "then" and (index == 0 or tokens[index - 1] != "elseif"):
            delta += 1
        elif token in {"end", "until"}:
            delta -= 1
    return delta


def test_blocks(source: str) -> list[str]:
    masked_lines = mask_lua_comments_and_strings(source).splitlines()
    original_lines = source.splitlines()
    blocks: list[str] = []
    index = 0
    while index < len(masked_lines):
        if TEST_START_RE.search(masked_lines[index]) is None:
            index += 1
            continue
        depth = block_delta(masked_lines[index])
        end = index
        while depth > 0 and end + 1 < len(masked_lines):
            end += 1
            depth += block_delta(masked_lines[end])
        blocks.append("\n".join(original_lines[index : end + 1]))
        index = end + 1
    return blocks


def trace_field_re(var: str) -> re.Pattern[str]:
    fields = "|".join(re.escape(field) for field in TRACE_FIELDS)
    return re.compile(
        r"\b" + re.escape(var) + r"\s*(?:\.\s*(?:" + fields + r")|\[\s*(?P<quote>[\"'])(?:"
        + fields + r")(?P=quote)\s*\])"
    )


def visible_regex_at(pattern: re.Pattern[str], masked: str, start: int) -> bool:
    return pattern.match(masked, start) is not None


def visible_fire_raiser_call(masked: str, match: re.Match[str]) -> bool:
    head = FIRE_RAISER_HEAD_RE.search(match.group(0))
    if head is None:
        return False
    return visible_regex_at(FIRE_RAISER_HEAD_RE, masked, match.start() + head.start())


def trace_field_spans(source: str, masked: str, var: str) -> list[tuple[int, int]]:
    spans: list[tuple[int, int]] = []
    for match in trace_field_re(var).finditer(source):
        if masked[match.start() : match.start() + len(var)].strip():
            spans.append((match.start(), match.end()))
    return spans


def matching_paren_end(masked: str, open_index: int) -> int:
    depth = 0
    for index in range(open_index, len(masked)):
        if masked[index] == "(":
            depth += 1
        elif masked[index] == ")":
            depth -= 1
            if depth == 0:
                return index + 1
    return len(masked)


def assertion_call_spans(source: str, masked: str) -> list[tuple[int, int]]:
    spans: list[tuple[int, int]] = []
    for match in ASSERTION_CALL_RE.finditer(source):
        if visible_regex_at(ASSERTION_CALL_RE, masked, match.start()):
            spans.append((match.start(), matching_paren_end(masked, match.end() - 1)))
    return spans


def span_inside(inner: tuple[int, int], outer: tuple[int, int]) -> bool:
    return outer[0] <= inner[0] and inner[1] <= outer[1]


def action_in_span(source: str, masked: str, start: int, end: int) -> bool:
    for match in IF_ASSERTION_ACTION_RE.finditer(source, start, end):
        if visible_regex_at(IF_ASSERTION_ACTION_RE, masked, match.start()):
            return True
    return False


def if_assertion_spans(source: str, masked: str, refs: list[tuple[int, int]]) -> bool:
    for match in IF_THEN_RE.finditer(source):
        if not visible_regex_at(IF_HEAD_RE, masked, match.start()):
            continue
        condition = (match.start("condition"), match.end("condition"))
        if not any(span_inside(ref, condition) for ref in refs):
            continue
        end_match = re.search(r"\bend\b", masked[match.end() :])
        body_end = len(masked) if end_match is None else match.end() + end_match.start()
        if action_in_span(source, masked, match.end(), body_end):
            return True
    return False


def assertion_contains_trace(source: str, masked: str, refs: list[tuple[int, int]]) -> bool:
    assertions = assertion_call_spans(source, masked)
    if any(any(span_inside(ref, assertion) for ref in refs) for assertion in assertions):
        return True
    return if_assertion_spans(source, masked, refs)


def call_asserts_trace(block: str, match: re.Match[str]) -> bool:
    source = strip_lua_comments(block)
    masked = mask_lua_comments_and_strings(block)
    var = match.group("var")
    if var is not None:
        return assertion_contains_trace(source, masked, trace_field_spans(source, masked, var))
    tail = source[match.end() : match.end() + 240]
    tail_match = re.search(r"^\s*\.\s*(?:" + "|".join(TRACE_FIELDS) + r")\b", tail)
    if tail_match is None:
        return False
    ref = (match.start(), match.end() + tail_match.end())
    return assertion_contains_trace(source, masked, [ref])


def embedded_fire_raiser_child_sources(source: str) -> list[str]:
    bodies: list[str] = []
    cursor = 0
    while cursor < len(source):
        if source.startswith("--", cursor):
            bracket = long_bracket_at(source, cursor + 2)
            if bracket is not None:
                opener_len, closer = bracket
                cursor = end_of_long_bracket(source, cursor + 2 + opener_len, closer)
            else:
                newline = source.find("\n", cursor)
                cursor = len(source) if newline == -1 else newline
            continue
        char = source[cursor]
        if char in ("'", '"'):
            cursor = end_of_quoted_string(source, cursor)
            continue
        if char == "[":
            bracket = long_bracket_at(source, cursor)
            if bracket is not None:
                opener_len, closer = bracket
                body_start = cursor + opener_len
                close_start = source.find(closer, body_start)
                body_end = len(source) if close_start == -1 else close_start
                end = len(source) if close_start == -1 else close_start + len(closer)
                prefix = source[max(0, cursor - 160) : cursor]
                if FIRE_RAISER_CHILD_PREFIX_RE.search(prefix) is not None:
                    bodies.append(source[body_start:body_end])
                cursor = end
                continue
        cursor += 1
    return bodies


def covered_raisers_in_source(source: str) -> set[str]:
    return set().union(*covered_raiser_tests_in_source(source).values())


def covered_raiser_tests_in_source(source: str) -> dict[str, set[str]]:
    covered: set[str] = set()
    by_fixture: dict[str, set[str]] = {}
    for candidate in [source, *embedded_fire_raiser_child_sources(source)]:
        for block in test_blocks(candidate):
            test_name_match = re.search(r"\b(test_[A-Za-z0-9_]+)\b", block)
            test_name = test_name_match.group(1) if test_name_match is not None else ""
            searchable = strip_lua_comments(block)
            masked = mask_lua_comments_and_strings(block)
            for match in FIRE_RAISER_RE.finditer(searchable):
                if visible_fire_raiser_call(masked, match) and call_asserts_trace(block, match):
                    raiser = match.group("raiser")
                    covered.add(raiser)
                    for token in fixture_tokens(test_name):
                        by_fixture.setdefault(token, set()).add(raiser)
    by_fixture.setdefault("", set()).update(covered)
    return by_fixture


def fixture_tokens(test_name: str) -> set[str]:
    words = re.findall(r"[A-Za-z0-9]+", test_name.lower())
    tokens = set(words)
    for start in range(len(words)):
        current: list[str] = []
        for word in words[start:]:
            current.append(word)
            tokens.add("_".join(current))
    return tokens


def package_test_coverage(package: Path) -> set[str]:
    by_fixture = package_test_fixture_coverage(package)
    return set().union(*by_fixture.values()) if by_fixture else set()


def package_test_fixture_coverage(package: Path) -> dict[str, set[str]]:
    tests = package / "tests"
    if not tests.exists():
        return {}
    covered: dict[str, set[str]] = {}
    for path in sorted(tests.rglob("*_test.lua")):
        if path.is_file():
            for fixture, raisers in covered_raiser_tests_in_source(path.read_text(encoding="utf-8")).items():
                covered.setdefault(fixture, set()).update(raisers)
    return covered


def bracket_body(source: str, start: int) -> str | None:
    cursor = start
    while cursor < len(source) and source[cursor] != "{":
        cursor += 1
    if cursor >= len(source):
        return None
    depth = 0
    body_start = cursor + 1
    while cursor < len(source):
        char = source[cursor]
        if char in ("'", '"'):
            cursor = end_of_quoted_string(source, cursor)
            continue
        if char == "[":
            bracket = long_bracket_at(source, cursor)
            if bracket is not None:
                opener_len, closer = bracket
                cursor = end_of_long_bracket(source, cursor + opener_len, closer)
                continue
        if char == "{":
            depth += 1
        elif char == "}":
            depth -= 1
            if depth == 0:
                return source[body_start:cursor]
        cursor += 1
    return None


def top_level_table_bodies(source: str) -> list[str]:
    bodies: list[str] = []
    cursor = 0
    while cursor < len(source):
        char = source[cursor]
        if char in ("'", '"'):
            cursor = end_of_quoted_string(source, cursor)
            continue
        if char == "[":
            bracket = long_bracket_at(source, cursor)
            if bracket is not None:
                opener_len, closer = bracket
                cursor = end_of_long_bracket(source, cursor + opener_len, closer)
                continue
        if char != "{":
            cursor += 1
            continue
        body = bracket_body(source, cursor)
        if body is None:
            cursor += 1
            continue
        bodies.append(body)
        cursor += len(body) + 2
    return bodies


def producer_liveness_contracts(path: Path, root: Path, package_root: Path | None = None) -> set[ProducerLivenessContract]:
    source = strip_lua_comments(path.read_text(encoding="utf-8"))
    function_match = re.search(r"\bfunction\s+M\s*\.\s*producer_liveness_contracts\s*\(", source)
    if function_match is None:
        return set()
    body = bracket_body(source, function_match.end())
    if body is None:
        return set()
    base = root / "packages" if package_root is None else package_root
    package = path.parent.relative_to(base).parts[0]
    contracts: set[ProducerLivenessContract] = set()
    for entry_body in top_level_table_bodies(body):
        if "runtime_gate" not in entry_body or "adversarial_fixture" not in entry_body:
            continue
        fields = {
            match.group("name"): match.group("value")
            for match in re.finditer(
                r"\b(?P<name>producer_id|trigger_source|runtime_gate|adversarial_fixture)\s*=\s*(?P<quote>[\"'])(?P<value>[A-Za-z0-9_.-]+)(?P=quote)",
                entry_body,
            )
        }
        if {"producer_id", "trigger_source", "runtime_gate", "adversarial_fixture"} <= set(fields):
            contracts.add(
                ProducerLivenessContract(
                    package=package,
                    producer_id=fields["producer_id"],
                    trigger_source=fields["trigger_source"],
                    runtime_gate=fields["runtime_gate"],
                    adversarial_fixture=fields["adversarial_fixture"].replace("-", "_").lower(),
                )
            )
    return contracts


def declared_liveness_contracts(root: Path, package_root: Path | None = None) -> set[ProducerLivenessContract]:
    packages = root / "packages" if package_root is None else package_root
    if not packages.exists():
        return set()
    return {
        contract
        for path in sorted(packages.glob("*/core.lua"))
        if path.is_file()
        for contract in producer_liveness_contracts(path, root, packages)
    }


def declared_raiser(path: Path, root: Path, package_root: Path | None = None) -> ProducerRaiser:
    base = root / "packages" if package_root is None else package_root
    package = path.parents[1].relative_to(base).parts[0]
    source = strip_lua_comments(path.read_text(encoding="utf-8"))
    name_match = RAISER_NAME_RE.search(source)
    name = name_match.group("name") if name_match is not None else path.stem
    produces = [match.group("queue") for match in PRODUCES_STRING_RE.finditer(source)]
    for table in PRODUCES_TABLE_RE.finditer(source):
        produces.extend(match.group("value") for match in STRING_RE.finditer(table.group("body")))
    return ProducerRaiser(package, name, "packages/" + path.relative_to(base).as_posix(), tuple(dict.fromkeys(produces)))


def declared_raisers(root: Path, package_root: Path | None = None) -> set[ProducerRaiser]:
    packages = root / "packages" if package_root is None else package_root
    if not packages.exists():
        return set()
    return {
        declared_raiser(path, root, packages)
        for path in sorted(packages.glob("*/raisers/*.lua"))
        if path.is_file()
    }


def load_allowlist(path: Path) -> set[str]:
    if not path.exists():
        return set()
    entries: set[str] = set()
    for number, raw in enumerate(path.read_text(encoding="utf-8").splitlines(), start=1):
        line = raw.strip()
        if not line or line.startswith("#"):
            continue
        if re.fullmatch(r"[A-Za-z0-9_.-]+\.[A-Za-z0-9_.-]+", line) is None:
            raise ValueError(f"invalid {ALLOWLIST} line {number}: {raw}")
        entries.add(line)
    return entries


def allowlist_at_dev_base(root: Path) -> tuple[str, set[str] | None]:
    try:
        status, shown = ratchet_base.file_at_base(root, ALLOWLIST)
        if status != "present":
            return status, None
        assert shown is not None
        return "present", {
            line.strip()
            for line in shown.splitlines()
            if line.strip() and not line.lstrip().startswith("#")
        }
    except Exception:
        return "unresolved", None


def ratchet_messages(
    raisers: set[ProducerRaiser],
    coverage_by_package: dict[str, set[str]],
    allowlist: set[str],
    base_allowlist: set[str] | None = None,
    fixture_coverage_by_package: dict[str, dict[str, set[str]]] | None = None,
    contracts: set[ProducerLivenessContract] | None = None,
) -> list[str]:
    messages: list[str] = []
    declared_by_key = {raiser.key(): raiser for raiser in raisers}
    covered = {
        raiser.key()
        for raiser in raisers
        if raiser.name in coverage_by_package.get(raiser.package, set())
    }
    uncovered = set(declared_by_key) - covered

    for key in sorted(uncovered - allowlist):
        messages.append(
            f"{declared_by_key[key].label()} lacks a trace-asserting fire_raiser test; add fire_raiser(\"{declared_by_key[key].name}\") with consumer_result/source_payload/raised/routed_to assertions or list existing debt in {ALLOWLIST}"
        )
    for key in sorted(allowlist - uncovered):
        detail = "is covered" if key in covered else "has no declared raiser"
        messages.append(f"{key} is listed in {ALLOWLIST} but {detail}; prune the stale entry")
    if base_allowlist is not None:
        for key in sorted(allowlist - base_allowlist):
            messages.append(f"{key} grows {ALLOWLIST} relative to dev; add a fire_raiser trace assertion instead")
    fixture_coverage_by_package = fixture_coverage_by_package or {}
    for contract in sorted(contracts or set()):
        matching_raisers = [
            raiser
            for raiser in sorted(raisers)
            if raiser.package == contract.package and contract.trigger_source in raiser.produces
        ]
        if not matching_raisers:
            messages.append(f"{contract.producer_id} declares producer-liveness trigger_source={contract.trigger_source} but no matching raiser exists")
            continue
        if len(matching_raisers) > 1:
            labels = ", ".join(raiser.key() for raiser in matching_raisers)
            messages.append(f"{contract.producer_id} declares producer-liveness trigger_source={contract.trigger_source} but multiple raisers produce it: {labels}")
            continue
        raiser = matching_raisers[0]
        if raiser.key() in allowlist:
            continue
        fixture_token = contract.adversarial_fixture
        if raiser.name not in fixture_coverage_by_package.get(contract.package, {}).get(fixture_token, set()):
            messages.append(
                f"{raiser.label()} is runtime-gated by {contract.runtime_gate}; add a trace-asserting fire_raiser(\"{raiser.name}\") test whose test name includes {fixture_token}"
            )
    return messages


def repository_messages(root: Path, package_root: Path | None = None) -> list[str]:
    packages = root / "packages" if package_root is None else package_root
    raisers = declared_raisers(root, packages)
    fixture_coverage = {
        package.name: package_test_fixture_coverage(package)
        for package in sorted(packages.iterdir())
        if package.is_dir()
    } if packages.exists() else {}
    coverage = {
        package: set().union(*by_fixture.values()) if by_fixture else set()
        for package, by_fixture in fixture_coverage.items()
    }
    allowlist = load_allowlist(root / ALLOWLIST)
    base_status, base_allowlist = allowlist_at_dev_base(root)
    messages: list[str] = []
    if base_status == "unresolved":
        messages.append("cannot resolve dev base allowlist to enforce shrink-only ratchet; ensure CI provides the dev ref")
    messages.extend(ratchet_messages(raisers, coverage, allowlist, base_allowlist, fixture_coverage, declared_liveness_contracts(root, packages)))
    return messages
