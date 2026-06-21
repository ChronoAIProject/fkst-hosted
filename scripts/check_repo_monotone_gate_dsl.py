"""Conformance guard for package-side monotone gate DSL definitions."""

from __future__ import annotations

import re
from dataclasses import dataclass
from pathlib import Path

import ratchet_base


ALLOWLIST = "migration/monotone-gate-dsl.allowlist"
PACKAGE_GLOB = "github-devloop*"
GATE_PARTS = ("core", "gates")
RAW_MODULES = {"std.devloop_state", "std.devloop_markers", "std.devloop_markers.facts"}
RAW_TOKENS = (
    "current_state",
    "current_entity_state",
    "pr_origin_fact",
    "_trusted_marker_comments",
    "_comment_body",
    "_comment_created_at",
    "fkst:github-devloop:state:v1",
    "fkst:github-devloop:pr-origin:v1",
)
DANGEROUS_GLOBALS = (
    "require",
    "debug",
    "getfenv",
    "setfenv",
    "load",
    "loadstring",
    "dofile",
    "loadfile",
    "_G",
    "_ENV",
    "rawget",
    "rawset",
    "rawequal",
    "setmetatable",
    "getmetatable",
    "package",
)
LUA_NAME = r"[A-Za-z_][A-Za-z0-9_]*"
REQUIRE_RE = re.compile(
    r"""\brequire\s*(?:\(\s*)?(?:"([A-Za-z0-9_.\-]+)"|'([A-Za-z0-9_.\-]+)'|\[(=*)\[([A-Za-z0-9_.\-]+)\]\3\])"""
)
GATE_MODULE_RE = re.compile(r"^core\.gates\.[A-Za-z_][A-Za-z0-9_]*$")
GATE_PATH_RE = re.compile(r"(?:^|[./\\])core[/\\]gates[/\\][A-Za-z_][A-Za-z0-9_]*(?:\.lua)?$")
GATE_REQUIRE_BINDING_RE = re.compile(
    r"""\b(?:local\s+)?(?P<name>[A-Za-z_][A-Za-z0-9_]*)\s*=\s*require\s*(?:\(\s*)?(?:"std\.devloop_gate"|'std\.devloop_gate'|\[(=*)\[std\.devloop_gate\]\2\])"""
)


@dataclass(frozen=True, order=True)
class Finding:
    path: str
    kind: str
    token: str
    line: int

    @classmethod
    def parse(cls, line: str) -> "Finding":
        parts = line.split("|")
        if len(parts) < 6:
            raise ValueError(f"invalid {ALLOWLIST} line: {line}")
        path, kind, token, line_part, issue, why = parts[:6]
        if not path.startswith("packages/github-devloop") or "/core/gates/" not in path or not path.endswith(".lua"):
            raise ValueError(f"invalid {ALLOWLIST} path: {line}")
        if kind not in {"require", "raw-token", "dangerous-global", "monkey-patch"}:
            raise ValueError(f"invalid {ALLOWLIST} kind: {line}")
        if not line_part.startswith("line="):
            raise ValueError(f"invalid {ALLOWLIST} line number: {line}")
        if re.fullmatch(r"issue=#?\d+", issue) is None:
            raise ValueError(f"invalid {ALLOWLIST} issue link: {line}")
        if not why.startswith("why=") or why == "why=":
            raise ValueError(f"invalid {ALLOWLIST} WHY: {line}")
        return cls(path=path, kind=kind, token=token, line=int(line_part.removeprefix("line=")))

    def key(self) -> tuple[str, str, str, str]:
        return self.path, self.kind, self.token, str(self.line)

    def label(self) -> str:
        return f"{self.path}:{self.line} {self.kind} {self.token}"


@dataclass(frozen=True, order=True)
class BypassFinding:
    path: str
    target: str
    line: int

    def label(self) -> str:
        return f"{self.path}:{self.line} loader-bypass {self.target}"


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


def _long_bracket_match(text: str, start: int) -> tuple[int, int, int] | None:
    match = re.match(r"\[(=*)\[", text[start:])
    if match is None:
        return None
    close = "]" + match.group(1) + "]"
    content_start = start + len(match.group(0))
    close_start = text.find(close, content_start)
    if close_start == -1:
        return len(text), content_start, len(text)
    return close_start + len(close), content_start, close_start


def lua_string_literals(text: str) -> list[tuple[str, int]]:
    literals: list[tuple[str, int]] = []
    cursor = 0
    while cursor < len(text):
        if text.startswith("--", cursor):
            long_comment = _long_bracket_match(text, cursor + 2)
            if long_comment is not None:
                cursor = long_comment[0]
                continue
            newline = text.find("\n", cursor)
            cursor = len(text) if newline == -1 else newline
            continue
        if text[cursor] in {"'", '"'}:
            end = _quoted_string_end(text, cursor)
            literals.append((text[cursor + 1:end - 1], cursor))
            cursor = end
            continue
        long_string = _long_bracket_match(text, cursor)
        if long_string is not None:
            end, content_start, content_end = long_string
            literals.append((text[content_start:content_end], cursor))
            cursor = end
            continue
        cursor += 1
    return literals


def literal_concat_bypass_findings(path: str, source: str) -> set[BypassFinding]:
    literals = lua_string_literals(source)
    findings: set[BypassFinding] = set()
    for start in range(0, len(literals)):
        joined = ""
        for end in range(start, min(len(literals), start + 8)):
            joined += literals[end][0]
            normalized = joined.replace("\\", "/")
            if GATE_MODULE_RE.search(joined) is not None or GATE_PATH_RE.search(normalized) is not None:
                findings.add(BypassFinding(path, joined, line_number(source, literals[start][1])))
                break
    return findings


def strip_lua_comments_and_strings(text: str) -> str:
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


def line_number(source: str, offset: int) -> int:
    return source.count("\n", 0, offset) + 1


def required_module(match: re.Match[str]) -> str:
    return next(group for group in (match.group(1), match.group(2), match.group(4)) if group is not None)


def lua_name_re(name: str) -> re.Pattern[str]:
    return re.compile(rf"(?<![A-Za-z0-9_]){re.escape(name)}(?![A-Za-z0-9_])")


def gate_aliases(source: str, stripped: str) -> dict[str, set[int]]:
    aliases: dict[str, set[int]] = {}
    for match in GATE_REQUIRE_BINDING_RE.finditer(source):
        name = match.group("name")
        if stripped[match.start("name"):match.start("name") + len(name)] != name:
            continue
        aliases.setdefault(name, set()).add(match.start("name"))
    return aliases


def monkey_patch_findings(path: str, source: str, stripped: str) -> set[Finding]:
    findings: set[Finding] = set()
    aliases = gate_aliases(source, stripped)
    for alias, binding_offsets in aliases.items():
        direct_reassign = re.compile(rf"(?<![A-Za-z0-9_]){re.escape(alias)}\s*=")
        for match in direct_reassign.finditer(stripped):
            if match.start() not in binding_offsets:
                findings.add(Finding(path, "monkey-patch", alias, line_number(source, match.start())))
        field_assignment = re.compile(
            rf"(?<![A-Za-z0-9_]){re.escape(alias)}\s*(?:\.|:)\s*{LUA_NAME}\s*="
        )
        for match in field_assignment.finditer(stripped):
            findings.add(Finding(path, "monkey-patch", alias, line_number(source, match.start())))
        bracket_assignment = re.compile(rf"(?<![A-Za-z0-9_]){re.escape(alias)}\s*\[[^\]]+\]\s*=")
        for match in bracket_assignment.finditer(stripped):
            findings.add(Finding(path, "monkey-patch", alias, line_number(source, match.start())))
        function_assignment = re.compile(
            rf"\bfunction\s+{re.escape(alias)}\s*(?:\.|:)\s*{LUA_NAME}\b"
        )
        for match in function_assignment.finditer(stripped):
            findings.add(Finding(path, "monkey-patch", alias, line_number(source, match.start())))
    direct_module_assignment = re.compile(
        rf"\bstd\s*\.\s*devloop_gate\s*(?:(?:\.|:)\s*{LUA_NAME}|\[[^\]]+\])?\s*="
    )
    for match in direct_module_assignment.finditer(stripped):
        findings.add(Finding(path, "monkey-patch", "std.devloop_gate", line_number(source, match.start())))
    require_result_assignment = re.compile(rf"^\s*\)?\s*(?:(?:\.|:)\s*{LUA_NAME}|\[[^\]]+\])?\s*=")
    for match in REQUIRE_RE.finditer(source):
        if required_module(match) != "std.devloop_gate":
            continue
        if stripped[match.start():match.start() + len("require")] != "require":
            continue
        if require_result_assignment.search(stripped[match.end():]):
            findings.add(Finding(path, "monkey-patch", "std.devloop_gate", line_number(source, match.start())))
    return findings


def gate_sources(root: Path) -> dict[str, str]:
    sources: dict[str, str] = {}
    for package in sorted((root / "packages").glob(PACKAGE_GLOB)):
      gate_root = package.joinpath(*GATE_PARTS)
      if not gate_root.is_dir():
        continue
      for path in sorted(gate_root.rglob("*.lua")):
        if path.is_file():
          sources[path.relative_to(root).as_posix()] = path.read_text(encoding="utf-8")
    return sources


def production_sources(root: Path) -> dict[str, str]:
    sources: dict[str, str] = {}
    for package in sorted((root / "packages").glob(PACKAGE_GLOB)):
      if not package.is_dir():
        continue
      for path in sorted(package.rglob("*.lua")):
        if not path.is_file():
          continue
        rel = path.relative_to(root).as_posix()
        parts = rel.split("/")
        if "/tests/" in f"/{rel}/" or "/core/gates/" in f"/{rel}/":
          continue
        sources[rel] = path.read_text(encoding="utf-8")
    return sources


def loader_scan_sources(root: Path) -> dict[str, str]:
    sources: dict[str, str] = {}
    for package in sorted((root / "packages").glob(PACKAGE_GLOB)):
      if not package.is_dir():
        continue
      for path in sorted(package.rglob("*.lua")):
        if not path.is_file():
          continue
        rel = path.relative_to(root).as_posix()
        if "/core/gates/" in f"/{rel}/":
          continue
        sources[rel] = path.read_text(encoding="utf-8")
    return sources


def source_findings(path: str, source: str) -> set[Finding]:
    findings: set[Finding] = set()
    stripped = strip_lua_comments_and_strings(source)
    for match in REQUIRE_RE.finditer(source):
        module = required_module(match)
        findings.add(Finding(path, "require", module, line_number(source, match.start())))
        if module in RAW_MODULES:
            findings.add(Finding(path, "require", module, line_number(source, match.start())))
    for token in RAW_TOKENS:
        start = 0
        while True:
            index = stripped.find(token, start)
            if index == -1:
                break
            findings.add(Finding(path, "raw-token", token, line_number(source, index)))
            start = index + len(token)
    for token in DANGEROUS_GLOBALS:
        for match in lua_name_re(token).finditer(stripped):
            findings.add(Finding(path, "dangerous-global", token, line_number(source, match.start())))
    findings.update(monkey_patch_findings(path, source, stripped))
    return findings


def loader_bypass_findings(root: Path) -> set[BypassFinding]:
    findings: set[BypassFinding] = set()
    for path, source in loader_scan_sources(root).items():
        stripped = strip_lua_comments_and_strings(source)
        for match in REQUIRE_RE.finditer(source):
            if stripped[match.start():match.start() + len("require")] != "require":
                continue
            module = required_module(match)
            if GATE_MODULE_RE.fullmatch(module) is not None:
                findings.add(BypassFinding(path, module, line_number(source, match.start())))
        for literal, offset in lua_string_literals(source):
            if GATE_MODULE_RE.fullmatch(literal) is not None or GATE_PATH_RE.search(literal) is not None:
                findings.add(BypassFinding(path, literal, line_number(source, offset)))
        findings.update(literal_concat_bypass_findings(path, source))
    return findings


def current_findings(root: Path) -> set[Finding]:
    found: set[Finding] = set()
    for path, source in gate_sources(root).items():
        found.update(source_findings(path, source))
    return found


def load_allowlist(path: Path) -> set[Finding]:
    if not path.exists():
        return set()
    return {
        Finding.parse(line.strip())
        for line in path.read_text(encoding="utf-8").splitlines()
        if line.strip() and not line.lstrip().startswith("#")
    }


def allowlist_at_dev_base(root: Path) -> tuple[str, set[Finding] | None]:
    try:
        status, shown = ratchet_base.file_at_base(root, ALLOWLIST)
        if status == "absent":
            return status, set()
        if status != "present":
            return status, None
        assert shown is not None
        return "present", {
            Finding.parse(line.strip())
            for line in shown.splitlines()
            if line.strip() and not line.lstrip().startswith("#")
        }
    except Exception:
        return "unresolved", None


def repository_messages(root: Path, enforce_base: bool = True) -> list[str]:
    current = current_findings(root)
    allowlist = load_allowlist(root / ALLOWLIST)
    messages: list[str] = []
    base_allowlist: set[Finding] | None = None
    if enforce_base:
        base_status, base_allowlist = allowlist_at_dev_base(root)
        if base_status == "unresolved":
            messages.append("cannot resolve dev base allowlist to enforce shrink-only ratchet; ensure CI provides the dev ref")
    for finding in sorted(current):
        if not any(entry.key() == finding.key() for entry in allowlist):
            messages.append(f"{finding.label()} is forbidden in a core/gates DSL definition; gate definitions are loaded by std.devloop_gate.load_gate with injected constructors, must not require modules, must not read raw marker/cursor helpers, and must stay pure positive data construction without reflection, loaders, metatables, raw table access, globals, or monkey-patching")
    for finding in sorted(loader_bypass_findings(root)):
        messages.append(f"{finding.label()} is forbidden; gate definitions must be loaded only through std.devloop_gate.load_gate so the restricted _ENV sandbox is authoritative")
    for entry in sorted(allowlist):
        if not any(finding.key() == entry.key() for finding in current):
            messages.append(f"{entry.label()} no longer matches monotone-gate-dsl debt; prune the stale entry")
    if base_allowlist is not None:
        for entry in sorted(allowlist):
            if not any(base.key() == entry.key() for base in base_allowlist):
                messages.append(f"{entry.label()} grows monotone-gate-dsl allowlist relative to dev; migrate to std.devloop_gate data specs instead")
    return messages
