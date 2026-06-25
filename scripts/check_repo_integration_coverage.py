#!/usr/bin/env python3
"""Cross-package run_graph integration coverage shrink-only ratchet."""

from __future__ import annotations

import json
import re
import argparse
import sys
from dataclasses import dataclass
from pathlib import Path
from typing import Any


ALLOWLIST = "migration/integration-edge-coverage.allowlist"
EXCLUSIONS = "migration/integration-edge-coverage.exclusions"
HOST_ALLOWLIST = ".fkst/conformance/integration-edge-coverage.allowlist"
HOST_EXCLUSIONS = ".fkst/conformance/integration-edge-coverage.exclusions"
LOCAL_SPEC_RE = re.compile(r"\blocal\s+spec\s*=\s*\{")
FIELD_RE_TEMPLATE = r"\b{field}\s*=\s*\{{"
STRING_RE = re.compile(r"(?P<quote>[\"'])(?P<value>[A-Za-z0-9_.-]+)(?P=quote)")
STRING_LITERAL_RE = re.compile(r"(?P<quote>[\"'])(?P<value>[^\"'\\]*(?:\\.[^\"'\\]*)*)(?P=quote)")
ASSERT_COVERS_RE = re.compile(r"\bgraph\s*\.\s*assert_covers\s*\(")
EDGE_RE = re.compile(
    r"^(?P<queue>[A-Za-z0-9_.-]+) -> (?P<consumer_pkg>[A-Za-z0-9_.-]+)\.(?P<consumer_dept>[A-Za-z0-9_.-]+)$"
)


@dataclass(frozen=True, order=True)
class DepartmentSpec:
    package: str
    department: str
    path: str
    root_kind: str
    consumes: tuple[str, ...]
    produces: tuple[str, ...]

    def consumer_id(self) -> str:
        return f"{self.package}.{self.department}"


@dataclass(frozen=True, order=True)
class Edge:
    queue: str
    producer_pkg: str
    producer_dept: str
    consumer_pkg: str
    consumer_dept: str
    owner_scope: str

    @property
    def edge_id(self) -> str:
        return f"{self.queue} -> {self.consumer_pkg}.{self.consumer_dept}"

    def as_report(self, status: str) -> dict[str, str]:
        return {
            "edge_id": self.edge_id,
            "queue": self.queue,
            "producer_pkg": self.producer_pkg,
            "producer_dept": self.producer_dept,
            "consumer_pkg": self.consumer_pkg,
            "consumer_dept": self.consumer_dept,
            "owner_scope": self.owner_scope,
            "status": status,
        }


@dataclass(frozen=True, order=True)
class Exclusion:
    edge: str
    reason: str
    owner: str
    review_by: str


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


def strip_lua_comments_and_strings(text: str) -> str:
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


def matching_table_end(masked: str, open_index: int) -> int | None:
    depth = 0
    for index in range(open_index, len(masked)):
        char = masked[index]
        if char == "{":
            depth += 1
        elif char == "}":
            depth -= 1
            if depth == 0:
                return index + 1
    return None


def matching_call_end(masked: str, open_index: int) -> int:
    depth = 0
    for index in range(open_index, len(masked)):
        char = masked[index]
        if char == "(":
            depth += 1
        elif char == ")":
            depth -= 1
            if depth == 0:
                return index + 1
    return len(masked)


def table_span(pattern: re.Pattern[str], masked: str, start: int = 0, end: int | None = None) -> tuple[int, int] | None:
    match = pattern.search(masked, start, len(masked) if end is None else end)
    if match is None:
        return None
    open_index = match.end() - 1
    close_index = matching_table_end(masked, open_index)
    if close_index is None:
        return None
    if end is not None and close_index > end:
        return None
    return open_index, close_index


def code_string_literals(source: str, masked: str, start: int, end: int) -> list[str]:
    values: list[str] = []
    for match in STRING_RE.finditer(source, start, end):
        if masked[match.start("quote")] == source[match.start("quote")]:
            values.append(match.group("value"))
    return values


def spec_field_values(source: str, masked: str, spec_span: tuple[int, int], field: str) -> tuple[str, ...]:
    field_re = re.compile(FIELD_RE_TEMPLATE.format(field=re.escape(field)))
    span = table_span(field_re, masked, spec_span[0], spec_span[1])
    if span is None:
        return ()
    return tuple(dict.fromkeys(code_string_literals(source, masked, span[0], span[1])))


def package_dirs(root: Path) -> list[Path]:
    packages = root / "packages"
    if not packages.exists():
        return []
    return [path for path in sorted(packages.iterdir()) if path.is_dir()]


def host_package_dirs(root: Path) -> list[Path]:
    roots = [root / "packages", root / ".fkst" / "local-packages"]
    packages: list[Path] = []
    seen: set[Path] = set()
    for package_root in roots:
        if not package_root.exists():
            continue
        for path in sorted(package_root.iterdir()):
            if not path.is_dir():
                continue
            resolved = path.resolve()
            if resolved in seen:
                continue
            seen.add(resolved)
            packages.append(path)
    return packages


def department_package_dirs(root: Path, platform_root: Path | None = None) -> list[tuple[Path, str]]:
    if platform_root is None:
        return [(path, "platform") for path in package_dirs(root)]

    host_packages = [(path, "host") for path in host_package_dirs(root)]
    platform_packages = [(path, "platform") for path in package_dirs(platform_root)]
    return host_packages + platform_packages


def package_for_queue(current_package: str, queue: str) -> str:
    return queue.split(".", 1)[0] if "." in queue else current_package


def normalize_queue(current_package: str, queue: str) -> str:
    return queue if "." in queue else f"{current_package}.{queue}"


def department_specs(root: Path, platform_root: Path | None = None) -> set[DepartmentSpec]:
    specs: set[DepartmentSpec] = set()
    for package, root_kind in department_package_dirs(root, platform_root):
        for path in sorted(package.glob("departments/*/main.lua")):
            source = path.read_text(encoding="utf-8")
            masked = strip_lua_comments_and_strings(source)
            spec_span = table_span(LOCAL_SPEC_RE, masked)
            if spec_span is None:
                continue
            specs.add(
                DepartmentSpec(
                    package=package.name,
                    department=path.parent.name,
                    path=department_spec_path(root, platform_root, package, path),
                    root_kind=root_kind,
                    consumes=spec_field_values(source, masked, spec_span, "consumes"),
                    produces=spec_field_values(source, masked, spec_span, "produces"),
                )
            )
    return specs


def department_spec_path(root: Path, platform_root: Path | None, package: Path, path: Path) -> str:
    roots = [
        ("packages", root / "packages"),
        (".fkst/local-packages", root / ".fkst" / "local-packages"),
    ]
    if platform_root is not None:
        roots.append(("platform:packages", platform_root / "packages"))
    for prefix, package_root in roots:
        try:
            return prefix + "/" + path.relative_to(package_root).as_posix()
        except ValueError:
            continue
    return path.as_posix()


def cross_package_edge_records(root: Path, platform_root: Path | None = None) -> set[Edge]:
    specs = department_specs(root, platform_root)
    producers_by_queue: dict[str, set[DepartmentSpec]] = {}
    for spec in specs:
        for queue in spec.produces:
            producers_by_queue.setdefault(normalize_queue(spec.package, queue), set()).add(spec)

    host_mode = platform_root is not None
    edges: set[Edge] = set()
    for spec in specs:
        for queue in spec.consumes:
            normalized = normalize_queue(spec.package, queue)
            for producer in producers_by_queue.get(normalized, set()):
                if producer.package == spec.package:
                    continue
                owner_scope = edge_owner_scope_for_specs(producer, spec)
                if host_mode and owner_scope == "platform-owned":
                    continue
                edges.add(
                    Edge(
                        queue=normalized,
                        producer_pkg=producer.package,
                        producer_dept=producer.department,
                        consumer_pkg=spec.package,
                        consumer_dept=spec.department,
                        owner_scope=owner_scope,
                    )
                )
    return edges


def edge_owner_scope_for_specs(producer: DepartmentSpec, consumer: DepartmentSpec) -> str:
    if producer.root_kind == "platform" and consumer.root_kind == "platform":
        return "platform-owned"
    return "host-owned"


def edge_owner_scope(producer_pkg: str, consumer_pkg: str, platform_packages: set[str]) -> str:
    if producer_pkg in platform_packages and consumer_pkg in platform_packages:
        return "platform-owned"
    return "host-owned"


def cross_package_edges(root: Path, platform_root: Path | None = None) -> set[str]:
    return {edge.edge_id for edge in cross_package_edge_records(root, platform_root)}


def run_graph_test_files(root: Path, platform_root: Path | None = None) -> list[Path]:
    files: list[Path] = []
    for package, root_kind in department_package_dirs(root, platform_root):
        if platform_root is not None and root_kind == "platform":
            continue
        files.extend(sorted(package.glob("tests/run_graph*.lua")))
    return sorted(files)


def observed_edges(root: Path, platform_root: Path | None = None) -> set[str]:
    edges: set[str] = set()
    for path in run_graph_test_files(root, platform_root):
        source = path.read_text(encoding="utf-8")
        masked = strip_lua_comments_and_strings(source)
        for match in ASSERT_COVERS_RE.finditer(masked):
            call_end = matching_call_end(masked, match.end() - 1)
            for string_match in STRING_LITERAL_RE.finditer(source, match.end(), call_end):
                edge = string_match.group("value")
                if " -> " in edge:
                    edges.add(edge)
    return edges


def load_jsonl_objects(path: Path, label: str) -> list[tuple[int, dict[str, Any]]]:
    if not path.exists():
        return []
    entries: list[tuple[int, dict[str, Any]]] = []
    for number, raw in enumerate(path.read_text(encoding="utf-8").splitlines(), start=1):
        line = raw.strip()
        if not line or line.startswith("#"):
            continue
        try:
            item = json.loads(line)
        except json.JSONDecodeError as exc:
            raise ValueError(f"invalid {label} JSON on line {number}: {exc.msg}") from exc
        if not isinstance(item, dict):
            raise ValueError(f"invalid {label} line {number}: expected JSON object")
        entries.append((number, item))
    return entries


def load_allowlist(path: Path) -> set[str]:
    entries: set[str] = set()
    for number, item in load_jsonl_objects(path, ALLOWLIST):
        edge = item.get("edge")
        reason = item.get("reason")
        if not isinstance(edge, str) or " -> " not in edge:
            raise ValueError(f"invalid {ALLOWLIST} line {number}: edge is required")
        if not isinstance(reason, str) or reason.strip() == "":
            raise ValueError(f"invalid {ALLOWLIST} line {number}: reason is required")
        entries.add(edge)
    return entries


def load_exclusions(path: Path) -> dict[str, Exclusion]:
    exclusions: dict[str, Exclusion] = {}
    for number, item in load_jsonl_objects(path, EXCLUSIONS):
        edge = item.get("edge")
        reason = item.get("reason")
        owner = item.get("owner")
        review_by = item.get("review_by")
        if not isinstance(edge, str) or " -> " not in edge:
            raise ValueError(f"invalid {EXCLUSIONS} line {number}: edge is required")
        if not isinstance(reason, str) or reason.strip() == "":
            raise ValueError(f"invalid {EXCLUSIONS} line {number}: reason is required")
        if not isinstance(owner, str) or owner.strip() == "":
            raise ValueError(f"invalid {EXCLUSIONS} line {number}: owner is required")
        if not isinstance(review_by, str) or not re.fullmatch(r"\d{4}-\d{2}-\d{2}", review_by):
            raise ValueError(f"invalid {EXCLUSIONS} line {number}: review_by is required as YYYY-MM-DD")
        exclusions[edge] = Exclusion(edge=edge, reason=reason, owner=owner, review_by=review_by)
    return exclusions


def ratchet_messages(
    edges: set[str],
    observed: set[str],
    allowlist: set[str],
    exclusions: dict[str, Exclusion] | None = None,
) -> list[str]:
    exclusions = exclusions or {}
    excluded_edges = set(exclusions)
    messages: list[str] = []
    uncovered = edges - observed
    for edge in sorted(allowlist & excluded_edges):
        messages.append(f"{edge} is listed in both integration coverage allowlist and exclusions; keep only one")
    for edge in sorted(uncovered - allowlist - excluded_edges):
        messages.append(
            f"new uncovered cross-package edge {edge}: add a run_graph test covering it (graph.assert_covers), shrink-only ratchet"
        )
    for edge in sorted(allowlist & observed):
        messages.append(f"stale: remove {edge}, now covered")
    for edge in sorted(allowlist - edges):
        messages.append(f"stale: {edge} no longer exists")
    for edge in sorted(excluded_edges - edges):
        messages.append(f"stale: excluded edge {edge} no longer exists")
    return messages


def parse_edge_id(edge_id: str) -> tuple[str, str, str]:
    match = EDGE_RE.match(edge_id)
    if match is None:
        return edge_id.split(" -> ", 1)[0] if " -> " in edge_id else edge_id, "", ""
    return match.group("queue"), match.group("consumer_pkg"), match.group("consumer_dept")


def report_for_edges(
    edges: set[str],
    observed: set[str],
    allowlist: set[str],
    exclusions: dict[str, Exclusion],
    platform_packages: set[str],
    edge_records: set[Edge] | None = None,
) -> list[dict[str, str]]:
    records_by_id = {edge.edge_id: edge for edge in (edge_records or set())}
    report: list[dict[str, str]] = []
    for edge_id in sorted(edges):
        if edge_id in observed:
            status = "covered"
        elif edge_id in allowlist:
            status = "uncovered-allowlisted"
        elif edge_id in exclusions:
            status = "excluded"
        else:
            status = "uncovered-UNLISTED"

        record = records_by_id.get(edge_id)
        if record is not None:
            report.append(record.as_report(status))
            continue

        queue, consumer_pkg, consumer_dept = parse_edge_id(edge_id)
        producer_pkg = package_for_queue(consumer_pkg, queue)
        owner_scope = edge_owner_scope(producer_pkg, consumer_pkg, platform_packages)
        report.append(
            {
                "edge_id": edge_id,
                "queue": queue,
                "producer_pkg": producer_pkg,
                "producer_dept": "",
                "consumer_pkg": consumer_pkg,
                "consumer_dept": consumer_dept,
                "owner_scope": owner_scope,
                "status": status,
            }
        )
    return report


def allowlist_default(root: Path, platform_root: Path | None) -> Path:
    return root / (HOST_ALLOWLIST if platform_root is not None else ALLOWLIST)


def exclusions_default(root: Path, platform_root: Path | None) -> Path:
    return root / (HOST_EXCLUSIONS if platform_root is not None else EXCLUSIONS)


def edge_report(
    root: Path,
    platform_root: Path | None = None,
    allowlist_path: Path | None = None,
    exclusions_path: Path | None = None,
) -> list[dict[str, str]]:
    edge_records = cross_package_edge_records(root, platform_root)
    specs = department_specs(root, platform_root)
    platform_packages = {spec.package for spec in specs if spec.root_kind == "platform"}
    return report_for_edges(
        {edge.edge_id for edge in edge_records},
        observed_edges(root, platform_root),
        load_allowlist(allowlist_path or allowlist_default(root, platform_root)),
        load_exclusions(exclusions_path or exclusions_default(root, platform_root)),
        platform_packages,
        edge_records=edge_records,
    )


def repository_messages(
    root: Path,
    platform_root: Path | None = None,
    allowlist_path: Path | None = None,
    exclusions_path: Path | None = None,
) -> list[str]:
    return ratchet_messages(
        cross_package_edges(root, platform_root),
        observed_edges(root, platform_root),
        load_allowlist(allowlist_path or allowlist_default(root, platform_root)),
        load_exclusions(exclusions_path or exclusions_default(root, platform_root)),
    )


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Check cross-package run_graph integration coverage.")
    parser.add_argument("--project-root", type=Path, default=Path.cwd())
    parser.add_argument("--platform-root", type=Path)
    parser.add_argument("--allowlist-path", type=Path)
    parser.add_argument("--exclusions-path", type=Path)
    parser.add_argument("--json", action="store_true")
    parser.add_argument("--write-report", type=Path)
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    root = args.project_root.resolve()
    platform_root = args.platform_root.resolve() if args.platform_root is not None else None
    allowlist_path = args.allowlist_path.resolve() if args.allowlist_path is not None else None
    exclusions_path = args.exclusions_path.resolve() if args.exclusions_path is not None else None
    report = edge_report(root, platform_root, allowlist_path, exclusions_path)
    if args.write_report is not None:
        args.write_report.parent.mkdir(parents=True, exist_ok=True)
        args.write_report.write_text(json.dumps(report, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    if args.json:
        print(json.dumps(report, indent=2, sort_keys=True))
    messages = repository_messages(root, platform_root, allowlist_path, exclusions_path)
    if messages:
        stream = sys.stderr if args.json else sys.stdout
        print("integration coverage check failed:", file=stream)
        for message in messages:
            print(f"  {message}", file=stream)
        return 1
    if not args.json:
        print("OK: integration coverage ratchet passed")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
