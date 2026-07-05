#!/usr/bin/env python3
"""Static routing ratchet for the thin github-devloop intake package."""

from __future__ import annotations

import argparse
import itertools
import json
import re
from dataclasses import dataclass
from pathlib import Path


INTAKE_PACKAGE = "github-devloop-intake"
POLICY_SLOT_MANIFEST = Path("scripts/intake_policy_slots.json")
POLICY_SLOT_SCHEMA = "fkst.package-topology-policy-slots.v1"
INTAKE_POLICY_SLOT = "intake-policy"
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


@dataclass(frozen=True)
class PolicyImplementation:
    package: str
    topology: str


@dataclass(frozen=True)
class PolicySlot:
    name: str
    consumer_queue: str
    implementations: tuple[PolicyImplementation, ...]

    @property
    def packages(self) -> set[str]:
        return {implementation.package for implementation in self.implementations}


@dataclass(frozen=True)
class LegalTopology:
    name: str
    excluded_packages: tuple[str, ...]


def load_policy_slots(root: Path) -> list[PolicySlot]:
    path = root / POLICY_SLOT_MANIFEST
    raw = json.loads(path.read_text(encoding="utf-8"))
    if raw.get("schema") != POLICY_SLOT_SCHEMA:
        raise ValueError(f"{POLICY_SLOT_MANIFEST} schema must be {POLICY_SLOT_SCHEMA!r}")
    slots_raw = raw.get("policy_slots")
    if not isinstance(slots_raw, list) or not slots_raw:
        raise ValueError(f"{POLICY_SLOT_MANIFEST} policy_slots must be a non-empty list")
    slots: list[PolicySlot] = []
    seen_slot_names: set[str] = set()
    for slot_raw in slots_raw:
        if not isinstance(slot_raw, dict):
            raise ValueError(f"{POLICY_SLOT_MANIFEST} policy_slots entries must be objects")
        name = required_string(slot_raw, "name")
        if name in seen_slot_names:
            raise ValueError(f"{POLICY_SLOT_MANIFEST} duplicate policy slot {name!r}")
        seen_slot_names.add(name)
        consumer_queue = required_string(slot_raw, "consumer_queue")
        implementations_raw = slot_raw.get("implementations")
        if not isinstance(implementations_raw, list) or len(implementations_raw) < 2:
            raise ValueError(f"{POLICY_SLOT_MANIFEST} slot {name!r} must declare at least two implementations")
        implementations: list[PolicyImplementation] = []
        seen_packages: set[str] = set()
        seen_topologies: set[str] = set()
        for implementation_raw in implementations_raw:
            if not isinstance(implementation_raw, dict):
                raise ValueError(f"{POLICY_SLOT_MANIFEST} slot {name!r} implementation entries must be objects")
            package = required_string(implementation_raw, "package")
            topology = required_string(implementation_raw, "topology")
            if package in seen_packages:
                raise ValueError(f"{POLICY_SLOT_MANIFEST} slot {name!r} duplicate package {package!r}")
            if topology in seen_topologies:
                raise ValueError(f"{POLICY_SLOT_MANIFEST} slot {name!r} duplicate topology {topology!r}")
            seen_packages.add(package)
            seen_topologies.add(topology)
            implementations.append(PolicyImplementation(package=package, topology=topology))
        slots.append(PolicySlot(name=name, consumer_queue=consumer_queue, implementations=tuple(implementations)))
    return slots


def required_string(raw: dict[str, object], key: str) -> str:
    value = raw.get(key)
    if not isinstance(value, str) or not value:
        raise ValueError(f"{POLICY_SLOT_MANIFEST} {key} must be a non-empty string")
    if not re.fullmatch(r"[A-Za-z0-9_.-]+", value):
        raise ValueError(f"{POLICY_SLOT_MANIFEST} {key} has invalid value {value!r}")
    return value


def policy_slot_messages(root: Path) -> list[str]:
    try:
        slots = load_policy_slots(root)
    except (OSError, json.JSONDecodeError, ValueError) as exc:
        return [f"{POLICY_SLOT_MANIFEST}: invalid policy-slot manifest: {exc}"]
    if not any(slot.name == INTAKE_POLICY_SLOT for slot in slots):
        return [f"{POLICY_SLOT_MANIFEST}: missing required policy slot {INTAKE_POLICY_SLOT!r}"]
    return []


def intake_policy_slot(root: Path) -> PolicySlot:
    for slot in load_policy_slots(root):
        if slot.name == INTAKE_POLICY_SLOT:
            return slot
    raise ValueError(f"{POLICY_SLOT_MANIFEST} missing required policy slot {INTAKE_POLICY_SLOT!r}")


def legal_topologies(root: Path) -> list[LegalTopology]:
    slots = load_policy_slots(root)
    topologies: list[LegalTopology] = []
    for selected in itertools.product(*(slot.implementations for slot in slots)):
        name_parts: list[str] = []
        excluded: set[str] = set()
        for slot, selected_implementation in zip(slots, selected):
            name_parts.append(selected_implementation.topology)
            for implementation in slot.implementations:
                if implementation.package != selected_implementation.package:
                    excluded.add(implementation.package)
        topologies.append(LegalTopology(name="+".join(name_parts), excluded_packages=tuple(sorted(excluded))))
    return topologies


def topology_exclusivity_messages(root: Path, loaded_packages: set[str]) -> list[str]:
    messages: list[str] = []
    for slot in load_policy_slots(root):
        loaded = sorted(slot.packages & loaded_packages)
        if len(loaded) != 1:
            messages.append(
                f"topology must load exactly one implementation of policy slot {slot.name!r}; "
                f"loaded {loaded or 'none'} from {sorted(slot.packages)}"
            )
    return messages


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


def candidate_consuming_packages(all_sources: list[Source], candidate_queue: str) -> dict[str, list[str]]:
    consumers: dict[str, list[str]] = {}
    for source in all_sources:
        if not is_production_department(source):
            continue
        package = source.package
        if package is None:
            continue
        for queue in spec_queues(source.text, "consumes"):
            if queue == candidate_queue:
                consumers.setdefault(package, []).append(source.relpath)
    return consumers


def candidate_consumer_messages(all_sources: list[Source], policy_slot: PolicySlot) -> list[str]:
    consumers = candidate_consuming_packages(all_sources, policy_slot.consumer_queue)
    consumer_packages = set(consumers)
    expected_packages = policy_slot.packages
    unexpected = consumer_packages - expected_packages
    if consumers and not unexpected:
        return []
    if not consumers:
        found = "found none"
    else:
        found = "found " + ", ".join(
            f"{package} ({', '.join(paths)})" for package, paths in sorted(consumers.items())
        )
    return [
        f"expected candidate-consuming production packages to be a non-empty subset of "
        f"{sorted(expected_packages)} for {policy_slot.consumer_queue}; {found}"
    ]


def repository_messages(root: Path) -> list[str]:
    all_sources = sources(root)
    messages = static_messages(all_sources)
    slot_messages = policy_slot_messages(root)
    messages.extend(slot_messages)
    if not slot_messages:
        messages.extend(candidate_consumer_messages(all_sources, intake_policy_slot(root)))
    return messages


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--topology-rows", metavar="ROOT")
    parser.add_argument("--assert-topology", metavar="ROOT")
    parser.add_argument("--packages", nargs="*", default=[])
    args = parser.parse_args()
    if args.topology_rows:
        for topology in legal_topologies(Path(args.topology_rows)):
            print(f"{topology.name}\t{','.join(topology.excluded_packages)}")
        return 0
    if args.assert_topology:
        messages = topology_exclusivity_messages(Path(args.assert_topology), set(args.packages))
        for message in messages:
            print(message)
        return 1 if messages else 0
    parser.error("expected --topology-rows or --assert-topology")
    return 2


if __name__ == "__main__":
    raise SystemExit(main())
