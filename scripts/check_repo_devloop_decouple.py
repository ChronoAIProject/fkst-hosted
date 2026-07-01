#!/usr/bin/env python3
"""G-DEVLOOP-DECOUPLE: a ratchet measuring how much production code still reads the devloop
god-table (libraries/devloop) through the ambient package table M.

It specifically closes the loophole the install_defs/m_writes/package_core_installs ratchet
left open: a copy-onto-M wrapper facade (`M.x = function(...) return mod.x(...) end`) drove
those counts to 0 WITHOUT decoupling. This counts the REAL coupling instead — reader
call-sites `(core|M).<devloop-symbol>(` in production code, excluding the wrapper defs (in
*/core.lua) and tests. A wrapper cannot lower it; only rewiring a reader to a direct
`require(module).fn(...)` call does. It is not a full proof of decoupling — a reader could
still reach the god-lib under a different local name or alternate import syntax — so it is a
shrink-only ratchet (progress lowers the committed baseline), not a "gaming is impossible"
claim. The endpoint is "no devloop god-table": every non-kernel devloop symbol is called
directly, and only a small documented version-CAS lifecycle kernel remains reachable through
the composed core.

Shrink-only against migration/devloop-decouple.inventory. Deterministic, read-only.
"""
import json
import re
import sys
from pathlib import Path

INVENTORY = "migration/devloop-decouple.inventory"
KERNEL_ALLOWLIST = "migration/devloop-decouple-kernel.allowlist"


def assembled_devloop_symbols(root: Path) -> set[str]:
    """The devloop god-table surface = every `M.<name> = ...` assembled in a package core."""
    symbols: set[str] = set()
    for core in root.glob("packages/*/core.lua"):
        for name in re.findall(r"^\s*M\.([A-Za-z_][A-Za-z0-9_]*)\s*=", core.read_text(encoding="utf-8"), re.M):
            symbols.add(name)
    return symbols


def kernel_allowlist(root: Path) -> set[str]:
    path = root / KERNEL_ALLOWLIST
    if not path.exists():
        return set()
    allow: set[str] = set()
    for line in path.read_text(encoding="utf-8").splitlines():
        line = line.split("#", 1)[0].strip()
        if line:
            allow.add(line)
    return allow


def reader_calls_through_m(root: Path, symbols: set[str]) -> int:
    """Count `(core|M).<symbol>(` reader call-sites in production code (excl. the wrapper
    definitions in */core.lua and excl. tests)."""
    if not symbols:
        return 0
    alt = "|".join(re.escape(s) for s in sorted(symbols, key=len, reverse=True))
    pattern = re.compile(rf"\b(?:core|M)\.(?:{alt})\s*\(")
    total = 0
    for lua in root.glob("packages/**/*.lua"):
        rel = lua.as_posix()
        if rel.endswith("/core.lua") or "/tests/" in rel:
            continue
        total += len(pattern.findall(lua.read_text(encoding="utf-8")))
    return total


def current_count(root: Path) -> int:
    symbols = assembled_devloop_symbols(root) - kernel_allowlist(root)
    return reader_calls_through_m(root, symbols)


def baseline(root: Path) -> int | None:
    path = root / INVENTORY
    if not path.exists():
        return None
    return int(json.loads(path.read_text(encoding="utf-8"))["reader_calls_through_m"])


def repository_messages(root: Path):
    """Yield violation messages (untagged; the runner adds the G-DEVLOOP-DECOUPLE tag)."""
    if not (root / "libraries" / "devloop").exists():
        return
    cur = current_count(root)
    base = baseline(root)
    if base is None:
        yield (
            f"missing baseline {INVENTORY}; create it with "
            f'{{"reader_calls_through_m": {cur}}} (shrink-only; progress lowers it toward the kernel)'
        )
        return
    if cur > base:
        yield (
            f"{cur} production reader-calls through the ambient M to devloop symbols "
            f"(baseline {base}); this GREW. A copy-onto-M wrapper cannot lower this — rewire "
            f"readers to direct require(module).fn(...) calls instead. Update {INVENTORY} only "
            f"when the real count drops."
        )


def check(root: Path, violations: list[str]) -> None:
    for message in repository_messages(root):
        violations.append(f"G-DEVLOOP-DECOUPLE: {message}")


if __name__ == "__main__":
    root = Path(sys.argv[1]) if len(sys.argv) > 1 else Path(".")
    v: list[str] = []
    check(root, v)
    print("current:", current_count(root), "baseline:", baseline(root))
    for m in v:
        print("VIOLATION:", m)
    sys.exit(1 if v else 0)
