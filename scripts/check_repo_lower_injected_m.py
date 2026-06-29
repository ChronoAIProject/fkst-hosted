#!/usr/bin/env python3
"""G-LOWER-INJECTED-M: shrink-only ratchet for lower-library injected-M coupling.

The harmful shape is lower libraries (`libraries/workflow`, `libraries/forge`) using
the composed package facade `M` that was injected as a function parameter. That
late-bound table lets lower libraries read product/devloop behavior without a typed
dependency. The dissolution target is zero injected-M member references in those
lower libraries.

The committed baseline lives in migration/lower-injected-m.inventory (JSON). CI fails
when any count grows above baseline, or when a current direct `M.<symbol>` /
`M:<symbol>` / `M[...]` reference has no declared route owner in the manifest.
Recording progress means lowering the counts and pruning the per-symbol manifest as
injected-M references are dissolved.
Baseline raises are still in-repository, review-visible diffs matching the repo's
other shrink-only ratchets: CI prevents silent growth, while a reviewer can see and
challenge any baseline increase.

This is a grep/regex line scanner, not a complete Lua data-flow scanner. It counts
the common statically-greppable direct forms inside any function body whose parameter
list contains bare `M`: dot/colon access, short bracket string literals (`M["foo"]`,
`M['foo']`), bracket expressions (`M[expr]` as `<dynamic>`), comparison/call
positions, and rebind RHS reads such as `local M = M.foo`. It does not count a
module's own top-level `local M = {}` table unless a function receives `M` as a
parameter. It cannot robustly handle Lua long-bracket-string index keys (`M[[foo]]`,
`M[=[foo]=]`), parenthesized access such as `(M).foo`, member access split across
physical lines, or fully-dynamic/aliased access such as `M[runtime_var]` and
`local t = M; t.foo`. It likewise cannot track lexical re-binding that shadows the
injected `M` inside a scanned body (`function f(M) local M = {...}; return M.x end`,
or `for _, M in ipairs(...)`): references after such a shadow read the new local, not
the injected parameter, but the regex scanner still attributes them to injected `M`.
Robust handling of these needs scope/data-flow tracking (the AST upgrade below).
Block scoping is token-based and targets common Lua block
forms; exotic multiline block opener forms remain within this scanner's documented
regex-limitation envelope. A Lua-AST/data-flow scanner is the future upgrade. The
route manifest, where every counted symbol declares a route owner, is the
authoritative defense; per-seam-slice grep review is defense in depth.
"""
from __future__ import annotations

import json
import re
from collections import Counter
from pathlib import Path

INVENTORY = "migration/lower-injected-m.inventory"
LIBRARIES = ("workflow", "forge")
COUNT_KEYS = (
    "workflow_injected_m_reads",
    "workflow_injected_m_unique_symbols",
    "forge_injected_m_reads",
    "forge_injected_m_unique_symbols",
    "total_injected_m_reads",
    "total_injected_m_unique_symbols",
)
ROUTES = {"contract", "workflow", "forge", "typed_port", "move_up"}

M_PARAMETER_FUNCTION_RE = re.compile(
    r"\bfunction\b(?:\s+[A-Za-z_][A-Za-z0-9_.:]*)?\s*\(([^)]*)\)"
)
LUA_WORD_RE = re.compile(r"\b[A-Za-z_][A-Za-z0-9_]*\b")
M_MEMBER_RE = re.compile(r"(?<![A-Za-z0-9_.])M\s*[.:]\s*([A-Za-z_][A-Za-z0-9_]*)")
M_BRACKET_START_RE = re.compile(r"(?<![A-Za-z0-9_.])M\s*\[")


def _mask(chars: list[str], start: int, end: int) -> None:
    for index in range(start, end):
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
    level = cursor - index - 1
    return cursor - index + 1, "]" + ("=" * level) + "]"


def _end_of_long_bracket(text: str, body_start: int, closer: str) -> int:
    close_start = text.find(closer, body_start)
    return len(text) if close_start == -1 else close_start + len(closer)


def _end_of_quoted_string(text: str, start: int) -> int:
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
    """Return code with comments and strings blanked while preserving offsets."""
    chars = list(text)
    cursor = 0
    while cursor < len(text):
        if text.startswith("--", cursor):
            bracket = _long_bracket_at(text, cursor + 2)
            if bracket is not None:
                opener_len, closer = bracket
                end = _end_of_long_bracket(text, cursor + 2 + opener_len, closer)
            else:
                newline = text.find("\n", cursor)
                end = len(text) if newline == -1 else newline
            _mask(chars, cursor, end)
            cursor = end
            continue

        char = text[cursor]
        if char in {"'", '"'}:
            end = _end_of_quoted_string(text, cursor)
            _mask(chars, cursor, end)
            cursor = end
            continue

        if char == "[":
            bracket = _long_bracket_at(text, cursor)
            if bracket is not None:
                opener_len, closer = bracket
                end = _end_of_long_bracket(text, cursor + opener_len, closer)
                _mask(chars, cursor, end)
                cursor = end
                continue

        cursor += 1
    return "".join(chars)


def block_delta(line: str) -> int:
    tokens = LUA_WORD_RE.findall(line)
    delta = 0
    loop_do_tokens = 0
    for token in tokens:
        if token in {"function", "if", "repeat"}:
            delta += 1
        elif token in {"for", "while"}:
            delta += 1
            loop_do_tokens += 1
        elif token == "do":
            if loop_do_tokens > 0:
                loop_do_tokens -= 1
            else:
                delta += 1
        elif token in {"end", "until"}:
            delta -= 1
    return delta


def _has_bare_m_parameter(parameter_list: str) -> bool:
    return any(part.strip() == "M" for part in parameter_list.split(","))


def _m_parameter_function_match(masked_line: str) -> re.Match[str] | None:
    for match in M_PARAMETER_FUNCTION_RE.finditer(masked_line):
        if _has_bare_m_parameter(match.group(1)):
            return match
    return None


def _install_block_spans(source: str) -> list[tuple[int, int, int]]:
    code_lines = lua_code_mask(source).splitlines()
    blocks: list[tuple[int, int, int]] = []
    index = 0
    while index < len(code_lines):
        match = _m_parameter_function_match(code_lines[index])
        if match is None:
            index += 1
            continue
        depth = block_delta(code_lines[index])
        end = index
        while depth > 0 and end + 1 < len(code_lines):
            end += 1
            depth += block_delta(code_lines[end])
        blocks.append((index + 1, end + 1, match.end()))
        index = end + 1
    return blocks


def install_blocks(source: str) -> list[tuple[int, int]]:
    return [(start, end) for start, end, _scan_start in _install_block_spans(source)]


def _matching_square_bracket(masked_line: str, open_index: int) -> int | None:
    depth = 0
    for index in range(open_index, len(masked_line)):
        char = masked_line[index]
        if char == "[":
            depth += 1
        elif char == "]":
            depth -= 1
            if depth == 0:
                return index
    return None


def _short_string_literal_value(expr: str) -> str | None:
    stripped = expr.strip()
    if len(stripped) < 2 or stripped[0] not in {"'", '"'}:
        return None
    quote = stripped[0]
    chars: list[str] = []
    cursor = 1
    while cursor < len(stripped):
        char = stripped[cursor]
        if char == "\\":
            if cursor + 1 >= len(stripped):
                return None
            chars.append(stripped[cursor : cursor + 2])
            cursor += 2
            continue
        if char == quote:
            return "".join(chars) if stripped[cursor + 1 :].strip() == "" else None
        chars.append(char)
        cursor += 1
    return None


def _direct_bracket_symbols(original_line: str, masked_line: str) -> list[str]:
    symbols: list[str] = []
    for match in M_BRACKET_START_RE.finditer(masked_line):
        open_index = masked_line.rfind("[", match.start(), match.end())
        close_index = _matching_square_bracket(masked_line, open_index)
        if close_index is None:
            symbols.append("<dynamic>")
            continue
        expr = original_line[open_index + 1 : close_index]
        symbols.append(_short_string_literal_value(expr) or "<dynamic>")
    return symbols


def injected_m_symbols(source: str) -> Counter[str]:
    masked_lines = lua_code_mask(source).splitlines()
    original_lines = source.splitlines()
    counts: Counter[str] = Counter()
    for start, end, scan_start in _install_block_spans(source):
        for line_number in range(start - 1, end):
            offset = scan_start if line_number == start - 1 else 0
            masked_line = masked_lines[line_number][offset:]
            original_line = original_lines[line_number][offset:]
            for match in M_MEMBER_RE.finditer(masked_line):
                counts[match.group(1)] += 1
            for symbol in _direct_bracket_symbols(original_line, masked_line):
                counts[symbol] += 1
    return counts


def _lua_files(base: Path) -> list[Path]:
    return sorted(path for path in base.rglob("*.lua") if path.is_file())


def measure(root: Path) -> dict:
    symbols: dict[str, Counter[str]] = {library: Counter() for library in LIBRARIES}
    for library in LIBRARIES:
        base = root / "libraries" / library
        if not base.is_dir():
            continue
        for path in _lua_files(base):
            text = path.read_text(encoding="utf-8", errors="replace")
            symbols[library].update(injected_m_symbols(text))

    workflow_total = sum(symbols["workflow"].values())
    forge_total = sum(symbols["forge"].values())
    workflow_unique = len(symbols["workflow"])
    forge_unique = len(symbols["forge"])
    all_symbols = set(symbols["workflow"]) | set(symbols["forge"])

    return {
        "workflow_injected_m_reads": workflow_total,
        "workflow_injected_m_unique_symbols": workflow_unique,
        "forge_injected_m_reads": forge_total,
        "forge_injected_m_unique_symbols": forge_unique,
        "total_injected_m_reads": workflow_total + forge_total,
        "total_injected_m_unique_symbols": len(all_symbols),
        "symbols": {
            "workflow": sorted(symbols["workflow"]),
            "forge": sorted(symbols["forge"]),
        },
        "symbol_counts": {
            "workflow": dict(sorted(symbols["workflow"].items())),
            "forge": dict(sorted(symbols["forge"].items())),
        },
    }


def current_symbols(current: dict) -> dict[str, list[str]]:
    raw = current.get("symbols")
    if isinstance(raw, dict):
        return {
            library: sorted(str(symbol) for symbol in raw.get(library, []) or [])
            for library in LIBRARIES
        }
    return {library: [] for library in LIBRARIES}


def load_inventory(root: Path) -> tuple[dict | None, dict]:
    path = root / INVENTORY
    if not path.is_file():
        return None, {}
    try:
        data = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, ValueError):
        return None, {}
    counts = data.get("counts", data)
    baseline = {key: int(counts.get(key, 0)) for key in COUNT_KEYS}
    manifest = data.get("manifest", {})
    return baseline, manifest if isinstance(manifest, dict) else {}


def _manifest_entry(manifest: dict, library: str, symbol: str) -> dict | None:
    section = manifest.get(library, {})
    if not isinstance(section, dict):
        return None
    entry = section.get(symbol)
    return entry if isinstance(entry, dict) else None


def manifest_messages(manifest: dict, symbols: dict[str, list[str]]) -> list[str]:
    messages: list[str] = []
    for library in LIBRARIES:
        current = set(symbols.get(library, []))
        section = manifest.get(library, {})
        if not isinstance(section, dict):
            section = {}
        for symbol in sorted(current):
            entry = _manifest_entry(manifest, library, symbol)
            if entry is None:
                messages.append(
                    f"{library} M.{symbol} has no declared route in {INVENTORY}; "
                    "declare route contract|workflow|forge|typed_port|move_up and owner before adding or retaining it"
                )
                continue
            route = entry.get("route")
            owner = entry.get("owner")
            if route not in ROUTES:
                messages.append(f"{library} M.{symbol} has invalid route {route!r} in {INVENTORY}")
            if not isinstance(owner, str) or owner == "":
                messages.append(f"{library} M.{symbol} has missing owner in {INVENTORY}")
        for stale in sorted(set(section) - current):
            messages.append(f"{library} M.{stale} is stale in {INVENTORY}; remove dissolved manifest entries")
    return messages


def ratchet_messages(current: dict, baseline: dict | None, manifest: dict, symbols: dict | None = None):
    if baseline is None:
        yield (
            "missing baseline " + INVENTORY + "; create it with current measured counts "
            "and a route manifest (shrink-only: growth is forbidden, progress lowers it)"
        )
        return

    for key in COUNT_KEYS:
        cur = int(current.get(key, 0))
        base = int(baseline.get(key, 0))
        if cur > base:
            yield (
                f"lower-library injected-M coupling grew: {key} {cur} > baseline {base}. "
                "Do not add new lower-library reads from the composed M table; route through "
                "contract, workflow/forge ownership, or typed ports and lower the baseline only after shrinkage."
            )

    yield from manifest_messages(manifest, symbols if symbols is not None else current_symbols(current))


def repository_messages(root: Path):
    """No-op in repositories without the lower libraries governed by this ratchet."""
    if not any((root / "libraries" / library).is_dir() for library in LIBRARIES):
        return
    current = measure(root)
    baseline, manifest = load_inventory(root)
    yield from ratchet_messages(current, baseline, manifest, current_symbols(current))


if __name__ == "__main__":
    import sys

    r = Path(sys.argv[1]) if len(sys.argv) > 1 else Path(__file__).resolve().parents[1]
    cur = measure(r)
    base, route_manifest = load_inventory(r)
    print("current:", json.dumps(cur, sort_keys=True))
    print("baseline:", json.dumps(base, sort_keys=True))
    msgs = list(ratchet_messages(cur, base, route_manifest, current_symbols(cur)))
    for msg in msgs:
        print("VIOLATION:", msg)
    sys.exit(1 if msgs else 0)
