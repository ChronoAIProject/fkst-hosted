#!/usr/bin/env python3
"""G-DEVLOOP-INSTALLER: a shrink-only ratchet measuring the OTHER half of the devloop ambient-M
god-table — the install(M) composed-core coupling that G-DEVLOOP-DECOUPLE does not see.

G-DEVLOOP-DECOUPLE counts the copy-onto-M FACADE (`M.fn = mod.fn` explicit bindings in package
cores). It is blind to the `install(M)` COMPOSED-CORE: package cores call
`require("devloop.commands").install(M)` / `require("devloop.logging").install(M)` /
`require("devloop.state").install(M)`, and those modules define `function M.<name>` INSIDE the
installer, so department reads `core.log_raise(...)` / `core.current_state(...)` are genuine
ambient-M god-table coupling that no explicit `M.name=` binding exists for — invisible to the
facade ratchet. This ratchet makes that coupling VISIBLE and drives it to zero as the composed
core is re-architected to explicit typed capability injection (caps.log.raise(...), etc.), per
docs/devloop-decouple-endpoint.md and the typed-DI SPEC.

It resolves each package core's `require("devloop.<mod>").install(M)` calls, collects the
`function M.<name>` symbols those modules (and, for aggregator modules, their listed submodules)
install onto M, and counts `(core|M).<symbol>(` reader call-sites in production code (excl.
*/core.lua and tests). Shrink-only against migration/devloop-installer.inventory. Deterministic,
read-only. Not a full proof (a reader could reach an installed method under another alias), so it
is a shrink-only ratchet, not a "gaming is impossible" claim.
"""
import json
import re
import sys
from pathlib import Path

INVENTORY = "migration/devloop-installer.inventory"

_INSTALL = re.compile(r'require\(\s*["\'](devloop\.[A-Za-z0-9_.]+)["\']\s*\)\.install\(\s*M\s*\)')
_SUBMOD = re.compile(r'["\'](devloop\.[A-Za-z0-9_./]+)["\']')
# Installer methods are declared both as `function M.name(...)` and as `M.name = ...`
# assignments (e.g. M.payload_field = logging.payload_field); catch both so a method installed
# via assignment is not silently uncounted (a false-negative that would hide coupling).
_M_METHOD = re.compile(
    r'^\s*(?:function M\.([A-Za-z_][A-Za-z0-9_]*)|M\.([A-Za-z_][A-Za-z0-9_]*)\s*=)', re.M
)


def _install_method_names(text: str) -> set[str]:
    return {(m.group(1) or m.group(2)) for m in _M_METHOD.finditer(text)}


def _module_path(root: Path, mod: str) -> Path:
    return root / "libraries" / (mod.replace(".", "/") + ".lua")


def installer_symbols(root: Path) -> set[str]:
    """Symbols installed onto M by the devloop modules that package cores `install(M)`.

    For each `require("devloop.<mod>").install(M)` in a package core, collect the `function M.<name>`
    definitions in that module; for aggregator modules (whose install loops over a list of
    submodules), also collect from every `"devloop.<submod>"` string the module references.
    """
    install_mods: set[str] = set()
    for core in root.glob("packages/*/core.lua"):
        for m in _INSTALL.finditer(core.read_text(encoding="utf-8")):
            install_mods.add(m.group(1))

    symbols: set[str] = set()
    for mod in install_mods:
        path = _module_path(root, mod)
        if not path.exists():
            continue
        text = path.read_text(encoding="utf-8")
        symbols.update(_install_method_names(text))
        # aggregator: pull submodules it references and collect their installed methods too
        for sub in _SUBMOD.findall(text):
            sub_path = _module_path(root, sub)
            if sub_path.exists():
                symbols.update(_install_method_names(sub_path.read_text(encoding="utf-8")))
    return symbols


def reader_calls(root: Path, symbols: set[str]) -> int:
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
    return reader_calls(root, installer_symbols(root))


def baseline(root: Path) -> int | None:
    path = root / INVENTORY
    if not path.exists():
        return None
    return int(json.loads(path.read_text(encoding="utf-8"))["installer_reads_through_m"])


def repository_messages(root: Path):
    if not (root / "libraries" / "devloop").exists():
        return
    cur = current_count(root)
    base = baseline(root)
    if base is None:
        yield (
            f"missing baseline {INVENTORY}; create it with "
            f'{{"installer_reads_through_m": {cur}}} (shrink-only; migrate install(M) composed-core '
            f"reads to explicit typed capability injection to lower it toward zero)"
        )
        return
    if cur > base:
        yield (
            f"{cur} production reader-calls through the ambient M to install(M) composed-core "
            f"symbols (baseline {base}); this GREW. Do not add new install(M) composed-core reads; "
            f"migrate readers to explicit capability handles (caps.log/state/egress). Update "
            f"{INVENTORY} only when the real count drops."
        )


def check(root: Path, violations: list[str]) -> None:
    for message in repository_messages(root):
        violations.append(f"G-DEVLOOP-INSTALLER: {message}")


if __name__ == "__main__":
    root = Path(sys.argv[1]) if len(sys.argv) > 1 else Path(".")
    v: list[str] = []
    check(root, v)
    print("current:", current_count(root), "baseline:", baseline(root))
    for m in v:
        print("VIOLATION:", m)
    sys.exit(1 if v else 0)
