#!/usr/bin/env python3
"""Broad ratchet for monotone lifecycle gate bypasses.

G-MONOTONE-GATE discovers every raw lifecycle cursor read in github-devloop*
production packages and shared devloop library lifecycle helpers, then requires
each occurrence to be classified. Legitimate current-routing reads live in the
shrink-only allowlist; monotone gates use reached() or another approved
milestone accessor instead.

G-MONOTONE-GATE v2 keys debt by the semantic bucket (path, enclosing function,
kind, canonical token) and compares COUNTS per bucket (multiplicity-preserving),
not by exact source line. Line numbers are diagnostics only. This intentionally
decouples the gate from pure code motion (inserting a require, rewriting
M.foo -> typed_alias.foo, reformatting) so a behaviour-preserving refactor no
longer false-flags growth. Accepted blind spot (by design, confirmed via
cross-model review): the gate permits count-preserving relocation of an existing
raw read WITHIN the same (path, enclosing function, kind, canonical token) bucket
-- it prevents increases in semantic raw-read debt, it does not police
intra-function control-flow relocation (that is covered by review and tests, not
the debt gate). A different token, function, kind, or path is a different bucket
and IS flagged; adding a duplicate read in an existing bucket IS growth.
"""

from __future__ import annotations

import json
import re
from collections import defaultdict
from dataclasses import dataclass
from pathlib import Path

import ratchet_base


MANIFEST = "migration/monotone-gate.inventory"
ALLOWLIST = "migration/monotone-gate.allowlist"
APPROVED_ACCESSORS = {"devloop.state.reached", "devloop.gate.holds", "reached", "holds"}
SURFACE_KINDS = {"monotone-gate", "visibility"}
PACKAGE_GLOB = "github-devloop*"
PHASES = (
    "thinking",
    "dependency_wait",
    "ready",
    "implementing",
    "awaiting-pr",
    "pr-open",
    "reviewing",
    "review-meta",
    "merge-ready",
    "merging",
    "merged",
    "closed-unmerged",
    "fixing",
    "impl-failed",
    "blocked",
)
PHASE_LITERAL = "|".join(re.escape(phase) for phase in PHASES)
CURSOR_RE = re.compile(r"\b(?:current_entity_state|current_state)\s*\(")
STATE_EQ_RE = re.compile(
    r"\.\s*state\s*==\s*(?P<quote1>['\"])(?P<phase1>" + PHASE_LITERAL + r")(?P=quote1)"
    r"|(?P<quote2>['\"])(?P<phase2>" + PHASE_LITERAL + r")(?P=quote2)\s*==\s*[^)\n]*\.\s*state"
)
DEVLOOP_ALIAS_RE = re.compile(
    r"^\s*local\s+(?P<alias>[A-Za-z_][A-Za-z0-9_]*)\s*=\s*require\s*\(\s*['\"]devloop\.[^'\"]+['\"]\s*\)"
)
FUNCTION_RE = re.compile(
    r"^\s*(?:local\s+)?function\s+"
    r"(?P<name>[A-Za-z_][A-Za-z0-9_]*(?:\s*[.:]\s*[A-Za-z_][A-Za-z0-9_]*)*)\s*\("
    r"|^\s*(?P<assign>[A-Za-z_][A-Za-z0-9_]*(?:\s*[.:]\s*[A-Za-z_][A-Za-z0-9_]*)*)\s*=\s*function\b"
)
LUA_WORD_RE = re.compile(r"\b[A-Za-z_][A-Za-z0-9_]*\b")
GATE_KIND_RE = re.compile(r"\bgate_kind\s*=\s*['\"]monotone_milestone['\"]")
RESPONSIBILITY_RE = re.compile(r"\bresponsibility_signature\s*\(")
STRING_FIELD_RE = re.compile(r"\b(?P<field>[A-Za-z_][A-Za-z0-9_]*)\s*=\s*(?P<quote>['\"])(?P<value>[^'\"]*)(?P=quote)")
IMPLEMENTATION_RE = re.compile(r"^(?P<path>packages/github-devloop[^/]*/[^:]+\.lua):(?P<function>[A-Za-z_][A-Za-z0-9_.:]*)$")
SURFACE_TABLE_PREFIX_RE = re.compile(r"^[A-Za-z_][A-Za-z0-9_]*\.")


@dataclass(frozen=True, order=True)
class Surface:
    path: str
    function: str
    kind: str
    gate_kind: str
    milestone_accessor: str
    milestone: str
    milestone_domain: str
    why: str


@dataclass(frozen=True, order=True)
class Violation:
    path: str
    surface: str
    kind: str
    token: str
    line: int

    @classmethod
    def parse(cls, line: str) -> "Violation":
        parts = line.split("|")
        if len(parts) < 7:
            raise ValueError(f"invalid {ALLOWLIST} line: {line}")
        path, surface, kind, token, line_part, issue, why = parts[:7]
        if not path.endswith((".lua", ".py")):
            raise ValueError(f"invalid {ALLOWLIST} path: {line}")
        if kind not in {"cursor-read", "state-equality"}:
            raise ValueError(f"invalid {ALLOWLIST} kind: {line}")
        if not line_part.startswith("line="):
            raise ValueError(f"invalid {ALLOWLIST} line number: {line}")
        if re.fullmatch(r"issue=#?\d+", issue) is None:
            raise ValueError(f"invalid {ALLOWLIST} issue link: {line}")
        if not why.startswith("why=") or why == "why=":
            raise ValueError(f"invalid {ALLOWLIST} WHY: {line}")
        return cls(path=path, surface=surface, kind=kind, token=token, line=int(line_part.removeprefix("line=")))

    def key(self) -> tuple[str, str, str, str, str]:
        return self.path, self.surface, self.kind, self.token, str(self.line)

    def canonical_key(self) -> tuple[str, str, str, str]:
        # Migration bridge for the no-growth-vs-dev comparison ONLY: this branch
        # moved std/devloop_* -> libraries/devloop/* (the stdlib split), but dev's
        # base allowlist still records the old std/devloop_ paths. Canonicalizing
        # old->new lets the no-growth check see a moved entry as the SAME debt
        # (not new growth). This is NOT a behavior shim: the branch allowlist is
        # already fully on the new paths; this only reconciles against the OLD dev
        # base during the rename window. Remove this once the rename has landed on
        # dev (then dev's base carries the new paths and the remap is a no-op).
        path = self.path
        if path.startswith("std/devloop_"):
            path = "libraries/devloop/" + path.removeprefix("std/devloop_")
        moved_paths = {
            "packages/github-devloop/core/doctor.lua": "packages/github-devloop-ops/core/doctor.lua",
            "packages/github-devloop/core/state_gap.lua": "packages/github-devloop-ops/core/state_gap.lua",
            "packages/github-devloop/departments/observability/census.lua": "packages/github-devloop-ops/departments/observability/census.lua",
            "packages/github-devloop/departments/observability/reaper.lua": "packages/github-devloop-ops/departments/observability/reaper.lua",
        }
        if path == "packages/github-devloop/core/dependencies.lua" and self.surface == "M.dependency_wait_fact":
            path = "packages/github-devloop-ops/core/dependency_wait.lua"
        else:
            path = moved_paths.get(path, path)
        if path == "packages/github-devloop-pr/departments/merge/main.lua":
            path = "packages/github-devloop-pr/core/merge_executor.lua"
        if path == "packages/github-devloop-intake-default/departments/intake_judge/main.lua":
            path = "packages/github-devloop-intake/departments/intake_judge/main.lua"
        return path, self.canonical_surface(), self.kind, self.canonical_token()

    def canonical_surface(self) -> str:
        return SURFACE_TABLE_PREFIX_RE.sub("", self.surface, count=1)

    def canonical_token(self) -> str:
        token = self.token.replace(" ", "")
        if self.kind == "cursor-read":
            if token in {"current_state(", "current_entity_state("}:
                return token
            if token in {"M.current_state(", "M.current_entity_state("}:
                return token.removeprefix("M.")
        return self.token

    def label(self) -> str:
        return f"{self.path}:{self.line} {self.surface} {self.kind} {self.token}"


@dataclass(frozen=True)
class Block:
    name: str
    start: int
    end: int
    source: str


def strip_lua_line_comment(line: str) -> str:
    quote = None
    escaped = False
    for index, char in enumerate(line):
        if quote is not None:
            if escaped:
                escaped = False
            elif char == "\\":
                escaped = True
            elif char == quote:
                quote = None
            continue
        if char in {"'", '"'}:
            quote = char
            continue
        if line.startswith("--", index):
            return line[:index]
    return line


def code_without_lua_line_comments(source: str) -> str:
    return "\n".join(strip_lua_line_comment(line) for line in source.splitlines())


def _mask(chars: list[str], start: int, end: int) -> None:
    for index in range(start, end):
        if chars[index] != "\n":
            chars[index] = " "


def _quoted_string_end(text: str, start: int) -> int:
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


def lua_code_mask(text: str) -> str:
    chars = list(text)
    cursor = 0
    while cursor < len(text):
        if text.startswith("--", cursor):
            newline = text.find("\n", cursor)
            end = len(text) if newline == -1 else newline
            _mask(chars, cursor, end)
            cursor = end
            continue
        if text[cursor] in {"'", '"'}:
            end = _quoted_string_end(text, cursor)
            _mask(chars, cursor, end)
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


def function_blocks(source: str) -> list[Block]:
    code_lines = lua_code_mask(source).splitlines()
    original_lines = source.splitlines()
    blocks: list[Block] = []
    index = 0
    while index < len(code_lines):
        match = FUNCTION_RE.match(code_lines[index])
        if match is None:
            index += 1
            continue
        depth = block_delta(code_lines[index])
        end = index
        while depth > 0 and end + 1 < len(code_lines):
            end += 1
            depth += block_delta(code_lines[end])
        name = (match.group("name") or match.group("assign") or "unknown").replace(" ", "")
        blocks.append(Block(name=name, start=index + 1, end=end + 1, source="\n".join(original_lines[index:end + 1])))
        index += 1
    return blocks


def surface_for_line(blocks: list[Block], line_number: int) -> str:
    containing = [block for block in blocks if block.start <= line_number <= block.end]
    if not containing:
        return "<top-level>"
    return max(containing, key=lambda block: block.start).name


def block_for_function(source: str, function_name: str) -> Block | None:
    wanted = function_name.split(".")[-1].split(":")[-1]
    for block in function_blocks(source):
        if block.name == function_name or block.name.split(".")[-1].split(":")[-1] == wanted:
            return block
    return None


def devloop_aliases(source: str) -> set[str]:
    aliases: set[str] = set()
    for line in code_without_lua_line_comments(source).splitlines():
        match = DEVLOOP_ALIAS_RE.match(line)
        if match is not None:
            aliases.add(match.group("alias"))
    return aliases


def cursor_pattern_for_aliases(aliases: set[str]) -> re.Pattern[str]:
    aliases = sorted(aliases)
    if not aliases:
        return CURSOR_RE
    alias_prefix = "|".join(re.escape(alias) for alias in aliases)
    return re.compile(r"\b(?:(?:" + alias_prefix + r"|M)\s*\.\s*)?(?:current_entity_state|current_state)\s*\(")


def cursor_pattern(source: str) -> re.Pattern[str]:
    return cursor_pattern_for_aliases(devloop_aliases(source))


def cursor_violation_token(raw: str, aliases: set[str]) -> str:
    token = raw.replace(" ", "")
    if token in {"current_state(", "current_entity_state("}:
        return token
    if token in {"M.current_state(", "M.current_entity_state("}:
        return token.removeprefix("M.")
    for alias in aliases:
        prefix = f"{alias}."
        if token.startswith(prefix) and token.removeprefix(prefix) in {"current_state(", "current_entity_state("}:
            return token.removeprefix(prefix)
    return raw.strip()


def responsibility_blocks(source: str) -> list[Block]:
    lines = source.splitlines()
    code_lines = code_without_lua_line_comments(source).splitlines()
    blocks: list[Block] = []
    index = 0
    while index < len(code_lines):
        if RESPONSIBILITY_RE.search(code_lines[index]) is None:
            index += 1
            continue
        depth = code_lines[index].count("(") + code_lines[index].count("{") - code_lines[index].count(")") - code_lines[index].count("}")
        end = index
        while depth > 0 and end + 1 < len(code_lines):
            end += 1
            depth += code_lines[end].count("(") + code_lines[end].count("{") - code_lines[end].count(")") - code_lines[end].count("}")
        source_block = "\n".join(lines[index:end + 1])
        if GATE_KIND_RE.search(source_block):
            blocks.append(Block(name="responsibility_signature", start=index + 1, end=end + 1, source=source_block))
        index = end + 1
    return blocks


def string_fields(source: str) -> dict[str, str]:
    return {match.group("field"): match.group("value") for match in STRING_FIELD_RE.finditer(source)}


def load_manifest(path: Path) -> tuple[list[Surface], list[str]]:
    if not path.exists():
        return [], [f"manifest-missing: {MANIFEST} is required"]
    surfaces: list[Surface] = []
    messages: list[str] = []
    for number, raw in enumerate(path.read_text(encoding="utf-8").splitlines(), start=1):
        stripped = raw.strip()
        if not stripped or stripped.startswith("#"):
            continue
        try:
            doc = json.loads(stripped)
        except json.JSONDecodeError as exc:
            messages.append(f"manifest-invalid-json: {MANIFEST}:{number}: {exc.msg}")
            continue
        surface = Surface(
            path=str(doc.get("path", "")),
            function=str(doc.get("function", "")),
            kind=str(doc.get("kind", "")),
            gate_kind=str(doc.get("gate_kind", "")),
            milestone_accessor=str(doc.get("milestone_accessor", "")),
            milestone=str(doc.get("milestone", "")),
            milestone_domain=str(doc.get("milestone_domain", "")),
            why=str(doc.get("why", "")),
        )
        if surface.kind not in SURFACE_KINDS:
            messages.append(f"manifest-invalid-kind: {surface.path} {surface.function}")
        if surface.gate_kind != "monotone_milestone":
            messages.append(f"manifest-invalid-gate-kind: {surface.path} {surface.function}")
        if surface.milestone_accessor not in APPROVED_ACCESSORS:
            messages.append(f"manifest-invalid-accessor: {surface.path} {surface.function} {surface.milestone_accessor}")
        if surface.milestone not in PHASES:
            messages.append(f"manifest-invalid-milestone: {surface.path} {surface.function} {surface.milestone}")
        if not surface.milestone_domain:
            messages.append(f"manifest-missing-domain: {surface.path} {surface.function}")
        if not surface.why:
            messages.append(f"manifest-missing-why: {surface.path} {surface.function}")
        surfaces.append(surface)
    return surfaces, messages


def is_cursor_definition(line: str, match_start: int) -> bool:
    declaration = FUNCTION_RE.match(lua_code_mask(line))
    if declaration is None:
        return False
    name = declaration.group("name")
    if name is None:
        return False
    basename = name.replace(" ", "").split(".")[-1].split(":")[-1]
    if basename not in {"current_state", "current_entity_state"}:
        return False
    name_start, name_end = declaration.span("name")
    return name_start <= match_start < name_end


def block_violations(path: str, surface: str, block: Block, aliases: set[str] | None = None) -> list[Violation]:
    violations: list[Violation] = []
    visible_aliases = aliases if aliases is not None else devloop_aliases(block.source)
    cursor_re = cursor_pattern_for_aliases(visible_aliases)
    for offset, line in enumerate(code_without_lua_line_comments(block.source).splitlines()):
        line_number = block.start + offset
        for match in cursor_re.finditer(line):
            if is_cursor_definition(line, match.start()):
                continue
            violations.append(Violation(path, surface, "cursor-read", cursor_violation_token(match.group(0), visible_aliases), line_number))
        for match in STATE_EQ_RE.finditer(line):
            phase = match.group("phase1") or match.group("phase2") or "state"
            violations.append(Violation(path, surface, "state-equality", phase, line_number))
    return violations


def source_violations(path: str, source: str) -> list[Violation]:
    blocks = function_blocks(source)
    violations: list[Violation] = []
    visible_aliases = devloop_aliases(source)
    cursor_re = cursor_pattern_for_aliases(visible_aliases)
    for line_number, line in enumerate(code_without_lua_line_comments(source).splitlines(), start=1):
        surface = surface_for_line(blocks, line_number)
        for match in cursor_re.finditer(line):
            if is_cursor_definition(line, match.start()):
                continue
            violations.append(Violation(path, surface, "cursor-read", cursor_violation_token(match.group(0), visible_aliases), line_number))
        for match in STATE_EQ_RE.finditer(line):
            phase = match.group("phase1") or match.group("phase2") or "state"
            violations.append(Violation(path, surface, "state-equality", phase, line_number))
    return violations


def production_sources(root: Path, package_roots: list[Path] | None = None) -> dict[str, str]:
    sources: dict[str, str] = {}
    for packages in (package_roots or [root / "packages"]):
        for package_root in sorted(packages.glob(PACKAGE_GLOB)):
            if not package_root.is_dir():
                continue
            for path in sorted(package_root.rglob("*.lua")):
                if not path.is_file():
                    continue
                if "tests" in path.relative_to(package_root).parts:
                    continue
                sources["packages/" + path.relative_to(packages).as_posix()] = path.read_text(encoding="utf-8")
    devloop_root = root / "libraries" / "devloop"
    if devloop_root.exists():
        for path in sorted(devloop_root.rglob("*.lua")):
            if not path.is_file():
                continue
            relative_parts = path.relative_to(devloop_root).parts
            if "tests" in relative_parts:
                continue
            sources[path.relative_to(root).as_posix()] = path.read_text(encoding="utf-8")
    return sources


def package_sources(root: Path) -> dict[str, str]:
    return production_sources(root)


def accessor_references(source: str, accessor: str) -> bool:
    basename = accessor.split(".")[-1]
    return re.search(r"\b" + re.escape(basename) + r"\s*\(", lua_code_mask(source)) is not None


def manifest_messages(root: Path, sources: dict[str, str]) -> list[str]:
    surfaces, messages = load_manifest(root / MANIFEST)
    for surface in surfaces:
        source = sources.get(surface.path)
        if source is None:
            messages.append(f"manifest-stale-path: {surface.path}")
            continue
        block = block_for_function(source, surface.function)
        if block is None:
            messages.append(f"manifest-stale-function: {surface.path} {surface.function}")
            continue
        if not accessor_references(block.source, surface.milestone_accessor):
            messages.append(f"manifest-unbound-accessor: {surface.path} {surface.function} does not reference {surface.milestone_accessor}")
        for violation in sorted(block_violations(surface.path, surface.function, block, devloop_aliases(source))):
            messages.append(f"{violation.label()} reads a transient cursor inside a declared monotone milestone surface; use {surface.milestone_accessor}")
    return messages


def responsibility_binding_messages(sources: dict[str, str]) -> list[str]:
    messages: list[str] = []
    for rel, source in sorted(sources.items()):
        for block in responsibility_blocks(source):
            fields = string_fields(block.source)
            accessor = fields.get("milestone_accessor", "")
            implementation = fields.get("milestone_implementation", "")
            if accessor not in APPROVED_ACCESSORS:
                messages.append(f"{rel}:{block.start} monotone_milestone responsibility_signature must declare an approved milestone_accessor")
            for violation in sorted(block_violations(rel, block.name, block, devloop_aliases(source))):
                messages.append(f"{violation.label()} reads a transient cursor inside monotone_milestone responsibility metadata")
            match = IMPLEMENTATION_RE.fullmatch(implementation)
            if match is None:
                messages.append(f"{rel}:{block.start} monotone_milestone responsibility_signature must bind milestone_implementation as packages/github-devloop*/...lua:function")
                continue
            impl_path = match.group("path")
            impl_function = match.group("function")
            impl_source = sources.get(impl_path)
            if impl_source is None:
                messages.append(f"{rel}:{block.start} monotone_milestone implementation path is stale: {impl_path}")
                continue
            impl_block = block_for_function(impl_source, impl_function)
            if impl_block is None:
                messages.append(f"{rel}:{block.start} monotone_milestone implementation function is stale: {implementation}")
                continue
            if not accessor_references(impl_block.source, accessor):
                messages.append(f"{rel}:{block.start} monotone_milestone implementation {implementation} does not reference {accessor}")
            for violation in sorted(block_violations(impl_path, impl_function, impl_block, devloop_aliases(impl_source))):
                messages.append(f"{violation.label()} reads a transient cursor inside monotone_milestone implementation {implementation}")
    return messages


def current_violations(root: Path, package_roots: list[Path] | None = None) -> tuple[list[Violation], list[str]]:
    sources = production_sources(root, package_roots)
    found: list[Violation] = []
    for path, source in sorted(sources.items()):
        found.extend(source_violations(path, source))
    messages = manifest_messages(root, sources)
    messages.extend(responsibility_binding_messages(sources))
    return found, messages


def load_allowlist(path: Path) -> list[Violation]:
    if not path.exists():
        return []
    return [
        Violation.parse(line.strip())
        for line in path.read_text(encoding="utf-8").splitlines()
        if line.strip() and not line.lstrip().startswith("#")
    ]


def allowlist_at_dev_base(root: Path) -> tuple[str, list[Violation] | None]:
    try:
        status, shown = ratchet_base.file_at_base(root, ALLOWLIST)
        if status != "present":
            return status, None
        assert shown is not None
        return "present", [
            Violation.parse(line.strip())
            for line in shown.splitlines()
            if line.strip() and not line.lstrip().startswith("#")
        ]
    except Exception:
        return "unresolved", None


def grouped_violations(violations: list[Violation] | set[Violation]) -> dict[tuple[str, str, str, str], list[Violation]]:
    grouped: dict[tuple[str, str, str, str], list[Violation]] = defaultdict(list)
    for violation in violations:
        grouped[violation.canonical_key()].append(violation)
    return {key: sorted(values) for key, values in grouped.items()}


def key_label(key: tuple[str, str, str, str]) -> str:
    path, surface, kind, token = key
    return f"{path} {surface} {kind} {token}"


def current_lines(values: list[Violation]) -> str:
    return f"current_lines={[violation.line for violation in values]}"


def ratchet_messages(
    current: list[Violation] | set[Violation],
    allowlist: list[Violation] | set[Violation],
    base_allowlist: list[Violation] | set[Violation] | None = None,
) -> list[str]:
    messages: list[str] = []
    current_by_key = grouped_violations(current)
    allowlist_by_key = grouped_violations(allowlist)
    base_by_key = grouped_violations(base_allowlist) if base_allowlist is not None else None
    for key, current_values in sorted(current_by_key.items()):
        allowed_count = len(allowlist_by_key.get(key, []))
        if len(current_values) > allowed_count:
            first = current_values[allowed_count]
            messages.append(
                f"{first.label()} is an unclassified transient lifecycle cursor read; {key_label(key)} has current_count={len(current_values)} allowed_count={allowed_count} {current_lines(current_values)}; migrate monotone gates to devloop.state.reached()/approved milestone accessors or classify legitimate current-routing debt in {ALLOWLIST}"
            )
    for key, allowlist_values in sorted(allowlist_by_key.items()):
        current_count = len(current_by_key.get(key, []))
        if current_count < len(allowlist_values):
            entry = allowlist_values[current_count]
            messages.append(
                f"{entry.label()} no longer matches monotone-gate debt; {key_label(key)} monotone-gate debt count shrank from allowed_count={len(allowlist_values)} to current_count={current_count} {current_lines(current_by_key.get(key, []))}; prune the stale entry"
            )
    if base_by_key is not None:
        for key, allowlist_values in sorted(allowlist_by_key.items()):
            base_count = len(base_by_key.get(key, []))
            if len(allowlist_values) > base_count:
                entry = allowlist_values[base_count]
                messages.append(
                    f"{entry.label()} grows monotone-gate allowlist relative to dev; {key_label(key)} allowlist_count={len(allowlist_values)} dev_count={base_count}; migrate to reached() instead"
                )
        for key, current_values in sorted(current_by_key.items()):
            base_count = len(base_by_key.get(key, []))
            if len(current_values) > base_count:
                first = current_values[base_count]
                messages.append(
                    f"{first.label()} grows monotone-gate debt relative to dev; {key_label(key)} current_count={len(current_values)} dev_count={base_count} {current_lines(current_values)}; migrate to reached() instead"
                )
    return messages


def repository_messages(root: Path, enforce_base: bool = True) -> list[str]:
    current, messages = current_violations(root)
    allowlist = load_allowlist(root / ALLOWLIST)
    base_allowlist: list[Violation] | None = None
    if enforce_base:
        base_status, base_allowlist = allowlist_at_dev_base(root)
        if base_status == "unresolved":
            messages.append("cannot resolve dev base allowlist to enforce shrink-only ratchet; ensure CI provides the dev ref")
    messages.extend(ratchet_messages(current, allowlist, base_allowlist))
    return messages
