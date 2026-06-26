#!/usr/bin/env python3
"""Static routing ratchet for the thin github-devloop intake package."""

from __future__ import annotations

import re
from dataclasses import dataclass
from pathlib import Path


INTAKE_PACKAGE = "github-devloop-intake"
CANDIDATE_QUEUE = "github-devloop-intake.devloop_intake_candidate"
LIFECYCLE_FORWARD_QUEUES = {
    "devloop_ready",
    "devloop_reviewing",
    "devloop_fixing",
    "devloop_merge_ready",
    "devloop_merge",
    "devloop_reconcile",
    "devloop_review_reconcile",
    "devloop_execute_request",
    "devloop_decompose",
    "devloop_liveness_tick",
}

SPEC_FIELD_RE = re.compile(r"\b(?P<field>consumes|produces)\s*=\s*\{(?P<body>.*?)\}", re.DOTALL)
RAISER_FIELD_RE = re.compile(r"\b(?P<field>type|produces)\s*=\s*(?P<quote>[\"'])(?P<value>[^\"']+)(?P=quote)")
LITERAL_RE = re.compile(r"(?P<quote>[\"'])(?P<value>[A-Za-z0-9_.-]+)(?P=quote)")
STATE_MARKER_CALL_RE = re.compile(r"\b(?:[A-Za-z_][A-Za-z0-9_]*\s*\.\s*)?state_marker\s*\(")
STATE_MARKER_LITERAL_RE = re.compile(r"fkst:github-devloop:state:v1|state:v1")
ISSUE_LIST_RE = re.compile(r"\bissue_list\b")


@dataclass(frozen=True)
class Source:
    relpath: str
    path: Path
    text: str

    @property
    def package(self) -> str | None:
        parts = Path(self.relpath).parts
        if len(parts) >= 2 and parts[0] == "packages":
            return parts[1]
        return None


def _mask(chars: list[str], start: int, end: int) -> None:
    for index in range(start, end):
        if chars[index] != "\n":
            chars[index] = " "


def lua_code_mask(text: str) -> str:
    chars = list(text)
    index = 0
    while index < len(text):
        if text.startswith("--", index):
            newline = text.find("\n", index)
            end = len(text) if newline == -1 else newline
            _mask(chars, index, end)
            index = end
            continue
        char = text[index]
        if char in {"'", '"'}:
            end = quoted_string_end(text, index)
            _mask(chars, index, end)
            index = end
            continue
        index += 1
    return "".join(chars)


def lua_string_literals(text: str) -> list[tuple[int, str]]:
    literals: list[tuple[int, str]] = []
    index = 0
    while index < len(text):
        if text.startswith("--", index):
            newline = text.find("\n", index)
            index = len(text) if newline == -1 else newline
            continue
        char = text[index]
        if char in {"'", '"'}:
            end = quoted_string_end(text, index)
            literals.append((index, text[index + 1:end - 1]))
            index = end
            continue
        index += 1
    return literals


def quoted_string_end(text: str, start: int) -> int:
    quote = text[start]
    index = start + 1
    while index < len(text):
        if text[index] == "\\":
            index += 2
            continue
        if text[index] == quote:
            return index + 1
        index += 1
    return len(text)


def line_number(text: str, index: int) -> int:
    return text.count("\n", 0, index) + 1


def spec_queues(source: str, field: str) -> list[str]:
    queues: list[str] = []
    for match in SPEC_FIELD_RE.finditer(source):
        if match.group("field") != field:
            continue
        for literal in LITERAL_RE.finditer(match.group("body")):
            queues.append(literal.group("value"))
    return queues


def raiser_fields(source: str) -> dict[str, str]:
    fields: dict[str, str] = {}
    for match in RAISER_FIELD_RE.finditer(source):
        fields[match.group("field")] = match.group("value")
    return fields


def queue_leaf(queue: str) -> str:
    return queue.rsplit(".", 1)[-1]


def is_state_marker_literal(value: str) -> bool:
    return "fkst:github-devloop:state:v1" in value or value == "state:v1"


def sources(root: Path) -> list[Source]:
    packages = root / "packages"
    if not packages.exists():
        return []
    result: list[Source] = []
    for path in sorted(packages.rglob("*.lua")):
        if not path.is_file():
            continue
        result.append(Source(path.relative_to(root).as_posix(), path, path.read_text(encoding="utf-8")))
    return result


def production_intake_sources(all_sources: list[Source]) -> list[Source]:
    result: list[Source] = []
    for source in all_sources:
        if source.package != INTAKE_PACKAGE:
            continue
        parts = Path(source.relpath).parts
        if "tests" in parts:
            continue
        result.append(source)
    return result


def is_intake_raiser(source: Source) -> bool:
    parts = Path(source.relpath).parts
    return len(parts) >= 4 and parts[:3] == ("packages", INTAKE_PACKAGE, "raisers")


def is_production_department(source: Source) -> bool:
    parts = Path(source.relpath).parts
    return len(parts) >= 5 and parts[0] == "packages" and parts[2] == "departments"


def static_messages(all_sources: list[Source]) -> list[str]:
    messages: list[str] = []
    for source in production_intake_sources(all_sources):
        if is_intake_raiser(source):
            fields = raiser_fields(source.text)
            raiser_type = fields.get("type", "unknown")
            messages.append(
                f"{source.relpath}: github-devloop-intake is event-driven only; no cron/file_watch raiser is allowed (type={raiser_type!r})"
            )
            continue

        for queue in spec_queues(source.text, "produces"):
            if queue == "consensus.proposal":
                messages.append(f"{source.relpath}: github-devloop-intake must not produce 'consensus.proposal'")
            elif queue_leaf(queue) in LIFECYCLE_FORWARD_QUEUES:
                messages.append(f"{source.relpath}: github-devloop-intake must not produce lifecycle queue {queue!r}")

        masked = lua_code_mask(source.text)
        for match in ISSUE_LIST_RE.finditer(masked):
            messages.append(
                f"{source.relpath}:{line_number(source.text, match.start())} github-devloop-intake must not self-read GitHub issue lists"
            )
        for match in STATE_MARKER_CALL_RE.finditer(masked):
            messages.append(
                f"{source.relpath}:{line_number(source.text, match.start())} github-devloop-intake must not build or write state:v1 markers"
            )
        for literal_start, value in lua_string_literals(source.text):
            if is_state_marker_literal(value):
                messages.append(
                    f"{source.relpath}:{line_number(source.text, literal_start)} github-devloop-intake must not build or write state:v1 markers"
                )
    return messages


def candidate_consuming_packages(all_sources: list[Source]) -> dict[str, list[str]]:
    consumers: dict[str, list[str]] = {}
    for source in all_sources:
        if not is_production_department(source):
            continue
        package = source.package
        if package is None:
            continue
        for queue in spec_queues(source.text, "consumes"):
            if queue == CANDIDATE_QUEUE:
                consumers.setdefault(package, []).append(source.relpath)
    return consumers


def candidate_consumer_messages(all_sources: list[Source]) -> list[str]:
    consumers = candidate_consuming_packages(all_sources)
    if len(consumers) == 1:
        return []
    if not consumers:
        found = "found none"
    else:
        found = "found " + ", ".join(
            f"{package} ({', '.join(paths)})" for package, paths in sorted(consumers.items())
        )
    return [
        f"expected exactly one package to consume {CANDIDATE_QUEUE}; {found}"
    ]


def repository_messages(root: Path) -> list[str]:
    all_sources = sources(root)
    return static_messages(all_sources) + candidate_consumer_messages(all_sources)
