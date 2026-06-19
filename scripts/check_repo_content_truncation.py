"""Shrink-only ratchet for lossy content truncation into payloads or prompts."""

from __future__ import annotations

import re
from dataclasses import dataclass
from pathlib import Path

import ratchet_base


ALLOWLIST = "migration/content-truncation.allowlist"
CONTENT_NAME_RE = re.compile(
    r"(?:body|bodies|comment|comments|context|content|diff|file|files|evidence|prompt|review|issue|notes)",
    re.IGNORECASE,
)
CAP_NAME_RE = re.compile(r"\b(?P<name>_?max_[A-Za-z0-9_]*(?:_len|_bytes))\b")
CAP_DECL_RE = re.compile(r"\b(?:local\s+)?(?P<name>_?max_[A-Za-z0-9_]*(?:_len|_bytes))\s*=\s*\d+\b")
MAX_FUNC_RE = re.compile(r"\b(?P<name>max_[A-Za-z0-9_]*(?:_len|_bytes))\s*\(\s*\)")
TRUNCATE_RE = re.compile(r"\b(?:truncate_utf8|bounded_text)\s*\(|:\s*sub\s*\(\s*1\s*,")
PROMPT_SINK_RE = re.compile(r"\b(?:spawn_codex(?:_sync)?|judgment_codex_opts|build_[A-Za-z0-9_]*prompt)\b")
RAISE_SINK_RE = re.compile(r"\b(?:raise|log_raise)\s*\(")
PAYLOAD_HINT_RE = re.compile(
    r"consensus\.proposal|consensus_reached|consensus_converge|github%-proxy\.(?:github_)?issue_create_request|github-proxy\.(?:github_)?issue_create_request"
)
CONTENT_FIELD_RE = re.compile(r"\b(?:body|context|content|content_fetch|comments?|diff|file|files|evidence|prompt)\s*=")
FUNCTION_RE = re.compile(
    r"^\s*(?:local\s+)?function\s+"
    r"(?P<name>[A-Za-z_][A-Za-z0-9_]*(?:\s*[.:]\s*[A-Za-z_][A-Za-z0-9_]*)*)\s*\("
    r"|^\s*(?P<assign>[A-Za-z_][A-Za-z0-9_]*(?:\s*[.:]\s*[A-Za-z_][A-Za-z0-9_]*)*)\s*=\s*function\b"
)
LUA_WORD_RE = re.compile(r"\b[A-Za-z_][A-Za-z0-9_]*\b")


@dataclass(frozen=True, order=True)
class ContentTruncationSite:
    path: str
    function: str
    cap: str
    sink: str
    line: int

    @classmethod
    def parse(cls, line: str) -> "ContentTruncationSite":
        parts = line.split("|")
        if len(parts) < 6:
            raise ValueError(f"invalid {ALLOWLIST} line: {line}")
        path, function, cap, sink, issue, why = parts[:6]
        if not path.startswith("packages/") or not path.endswith(".lua"):
            raise ValueError(f"invalid {ALLOWLIST} path: {line}")
        if not function:
            raise ValueError(f"invalid {ALLOWLIST} function: {line}")
        if CAP_NAME_RE.fullmatch(cap) is None and MAX_FUNC_RE.fullmatch(cap + "()") is None:
            raise ValueError(f"invalid {ALLOWLIST} cap: {line}")
        if sink not in {"raise-payload", "codex-prompt"}:
            raise ValueError(f"invalid {ALLOWLIST} sink: {line}")
        if re.fullmatch(r"issue=#?\d+", issue) is None:
            raise ValueError(f"invalid {ALLOWLIST} issue link: {line}")
        if not why.startswith("why=") or why == "why=":
            raise ValueError(f"invalid {ALLOWLIST} WHY: {line}")
        return cls(path=path, function=function, cap=cap, sink=sink, line=0)

    def allowlist_line(self, issue: int, why: str) -> str:
        return "|".join((self.path, self.function, self.cap, self.sink, f"issue=#{issue}", f"why={why}"))

    def key(self) -> tuple[str, str, str, str]:
        return self.path, self.function, self.cap, self.sink

    def label(self) -> str:
        return f"{self.path}:{self.line} {self.function} {self.cap} -> {self.sink}"


@dataclass(frozen=True)
class FunctionBlock:
    name: str
    start: int
    end: int
    source: str


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
            end = _quoted_string_end(text, index)
            _mask(chars, index, end)
            index = end
            continue
        index += 1
    return "".join(chars)


def _mask(chars: list[str], start: int, end: int) -> None:
    for index in range(start, end):
        if chars[index] != "\n":
            chars[index] = " "


def _quoted_string_end(text: str, start: int) -> int:
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


def function_blocks(source: str) -> list[FunctionBlock]:
    masked_lines = lua_code_mask(source).splitlines()
    original_lines = source.splitlines()
    blocks: list[FunctionBlock] = []
    index = 0
    while index < len(masked_lines):
        match = FUNCTION_RE.match(masked_lines[index])
        if match is None:
            index += 1
            continue
        depth = block_delta(masked_lines[index])
        end = index
        while depth > 0 and end + 1 < len(masked_lines):
            end += 1
            depth += block_delta(masked_lines[end])
        name = (match.group("name") or match.group("assign") or "unknown").replace(" ", "")
        blocks.append(FunctionBlock(name=name, start=index + 1, end=end + 1, source="\n".join(original_lines[index:end + 1])))
        index = end + 1
    return blocks


def package_lua_sources(root: Path, packages: Path, read_text, rel) -> dict[str, str]:
    if not packages.exists():
        return {}
    sources: dict[str, str] = {}
    for path in sorted(packages.rglob("*.lua")):
        if not path.is_file() or "tests" in path.relative_to(packages).parts:
            continue
        sources[rel(root, path)] = read_text(path)
    return sources


def cap_names(source: str) -> set[str]:
    names = {match.group("name") for match in CAP_DECL_RE.finditer(source)}
    names.update(match.group("name") for match in MAX_FUNC_RE.finditer(source))
    return {name for name in names if CONTENT_NAME_RE.search(name)}


def truncation_cap_on_line(line: str, caps: set[str]) -> str | None:
    if TRUNCATE_RE.search(line) is None:
        return None
    for cap in sorted(caps, key=len, reverse=True):
        if re.search(r"\b" + re.escape(cap) + r"\b", line) is not None:
            return cap
        if re.search(r"\b" + re.escape(cap) + r"\s*\(\s*\)", line) is not None:
            return cap
    return None


def sink_for_block(block_source: str, truncation_index: int) -> str | None:
    tail_lines = block_source.splitlines()[truncation_index:]
    tail = "\n".join(tail_lines[:80])
    if PROMPT_SINK_RE.search(tail) is not None:
        return "codex-prompt"
    if PAYLOAD_HINT_RE.search(tail) is not None and CONTENT_FIELD_RE.search(tail) is not None:
        return "raise-payload"
    if RAISE_SINK_RE.search(tail) is not None and (PAYLOAD_HINT_RE.search(tail) is not None or CONTENT_FIELD_RE.search(tail) is not None):
        return "raise-payload"
    return None


def block_sites(path: str, block: FunctionBlock, caps: set[str]) -> set[ContentTruncationSite]:
    sites: set[ContentTruncationSite] = set()
    lines = lua_code_mask(block.source).splitlines()
    for index, line in enumerate(lines):
        cap = truncation_cap_on_line(line, caps)
        if cap is None:
            continue
        sink = sink_for_block(block.source, index)
        if sink is None:
            continue
        sites.add(ContentTruncationSite(path=path, function=block.name, cap=cap, sink=sink, line=block.start + index))
    return sites


def truncation_candidates(path: str, block: FunctionBlock, caps: set[str]) -> set[ContentTruncationSite]:
    candidates: set[ContentTruncationSite] = set()
    lines = lua_code_mask(block.source).splitlines()
    for index, line in enumerate(lines):
        cap = truncation_cap_on_line(line, caps)
        if cap is not None:
            candidates.add(ContentTruncationSite(path=path, function=block.name, cap=cap, sink="", line=block.start + index))
    return candidates


def block_calls_function(block: FunctionBlock, function_name: str) -> bool:
    basename = function_name.split(".")[-1].split(":")[-1]
    return re.search(r"\b" + re.escape(basename) + r"\s*\(", lua_code_mask(block.source)) is not None


def source_sites(path: str, source: str) -> set[ContentTruncationSite]:
    caps = cap_names(source)
    if not caps:
        return set()
    sites: set[ContentTruncationSite] = set()
    blocks = function_blocks(source)
    candidates: set[ContentTruncationSite] = set()
    for block in blocks:
        sites.update(block_sites(path, block, caps))
        candidates.update(truncation_candidates(path, block, caps))
    for candidate in candidates:
        if candidate.sink != "":
            continue
        for block in blocks:
            if block.name == candidate.function or not block_calls_function(block, candidate.function):
                continue
            sink = sink_for_block(block.source, 0)
            if sink is not None:
                sites.add(ContentTruncationSite(
                    path=candidate.path,
                    function=candidate.function,
                    cap=candidate.cap,
                    sink=sink,
                    line=candidate.line,
                ))
                break
    return sites


def sites(sources: dict[str, str]) -> set[ContentTruncationSite]:
    result: set[ContentTruncationSite] = set()
    for path, source in sorted(sources.items()):
        result.update(source_sites(path, source))
    return result


def load_allowlist(path: Path) -> set[ContentTruncationSite]:
    if not path.exists():
        return set()
    entries: set[ContentTruncationSite] = set()
    for raw in path.read_text(encoding="utf-8").splitlines():
        stripped = raw.strip()
        if not stripped or stripped.startswith("#"):
            continue
        entry = ContentTruncationSite.parse(stripped)
        entries.add(entry)
    return entries


def covered_by_allowlist(site: ContentTruncationSite, allowlist: set[ContentTruncationSite]) -> bool:
    return any(entry.key() == site.key() for entry in allowlist)


def ratchet_messages(
    current: set[ContentTruncationSite],
    allowlist: set[ContentTruncationSite],
    base_allowlist: set[ContentTruncationSite] | None = None,
) -> list[str]:
    messages: list[str] = []
    for site in sorted(current):
        if not covered_by_allowlist(site, allowlist):
            messages.append(
                f"{site.label()} truncates large content into a reliable payload or codex prompt; use source_ref/content_fetch rehydration instead"
            )
    for entry in sorted(allowlist):
        if not any(site.key() == entry.key() for site in current):
            messages.append(f"{entry.path} {entry.function} {entry.cap} -> {entry.sink} no longer matches content-truncation debt; prune the stale entry")
    if base_allowlist is not None:
        for entry in sorted(allowlist):
            if not covered_by_allowlist(entry, base_allowlist):
                messages.append(f"{entry.path} {entry.function} {entry.cap} -> {entry.sink} grows content-truncation allowlist relative to dev; migrate to source_ref/content_fetch instead")
    return messages


def allowlist_at_dev_base(root: Path) -> tuple[str, set[ContentTruncationSite] | None]:
    try:
        status, shown = ratchet_base.file_at_base(root, ALLOWLIST)
        if status != "present":
            return status, None
        assert shown is not None
        return "present", {
            ContentTruncationSite.parse(line.strip())
            for line in shown.splitlines()
            if line.strip() and not line.lstrip().startswith("#")
        }
    except Exception:
        return "unresolved", None


def repository_messages(root: Path, packages: Path, read_text, rel) -> list[str]:
    current = sites(package_lua_sources(root, packages, read_text, rel))
    allowlist = load_allowlist(root / ALLOWLIST)
    base_status, base_allowlist = allowlist_at_dev_base(root)
    messages: list[str] = []
    if base_status == "unresolved":
        messages.append("cannot resolve dev base allowlist to enforce shrink-only ratchet; ensure CI provides the dev ref")
    messages.extend(ratchet_messages(current, allowlist, base_allowlist))
    return messages
