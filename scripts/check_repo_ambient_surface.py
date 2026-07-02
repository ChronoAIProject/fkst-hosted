#!/usr/bin/env python3
"""G-DEVLOOP-AMBIENT-SURFACE: shrink-only ratchet driving the install(M) ambient surface to zero.

The SECOND of the two DI-refactor ratchets (see
docs/superpowers/specs/2026-07-02-di-refactor-retire-ambient-m-design.md). Where
G-DEVLOOP-SERVICE-LOCATOR counts the READ side (department require("core") + core.X), this counts
the IMPLEMENTATION side — the ambient composed-M surface that `require("devloop.X").install(M)`
creates:

- `require("devloop.<mod>").install(M)` calls in package cores, and
- the exported method surface those devloop modules bind onto M (both `M.name = C.name`
  assignments and the loop-binding `for _,n in ipairs({...names...}) do M[n]=C[n] end` name lists).

The two ratchets together defeat ratchet-gaming: a prior audit caught the read-only migration
(installer-reads 1091->45) leaving the ambient surface fully intact. Moving reads core.X -> caps.X
lowers the read ratchet but NOT this one; only deleting the install(M) scaffolds (once nothing
consumes the ambient M) lowers this surface toward zero — genuine dissolution.

Shrink-only against migration/ambient-surface.inventory. Deterministic, read-only.
"""
import json
import re
import sys
from pathlib import Path

INVENTORY = "migration/ambient-surface.inventory"

_INSTALL = re.compile(r'require\(\s*["\'](devloop\.[A-Za-z0-9_.]+)["\']\s*\)\.install\(\s*M\s*\)')
_SUBMOD = re.compile(r'["\'](devloop\.[A-Za-z0-9_./]+)["\']')
_M_METHOD = re.compile(
    r'^\s*(?:function M\.([A-Za-z_][A-Za-z0-9_]*)|M\.([A-Za-z_][A-Za-z0-9_]*)\s*=)', re.M
)
_LOOP_BIND = re.compile(r'ipairs\(\s*\{([^}]*)\}\s*\)\s*do\s*M\[[A-Za-z_][A-Za-z0-9_]*\]\s*=')
_QUOTED = re.compile(r'["\']([A-Za-z_][A-Za-z0-9_]*)["\']')


def _module_path(root: Path, mod: str) -> Path:
    return root / "libraries" / (mod.replace(".", "/") + ".lua")


def _export_names(text: str) -> set:
    names = {(m.group(1) or m.group(2)) for m in _M_METHOD.finditer(text)}
    for loop in _LOOP_BIND.finditer(text):
        names.update(_QUOTED.findall(loop.group(1)))
    names.discard("install")
    return names


def counts(root: Path) -> dict:
    install_mods: set = set()
    install_calls = 0
    for core in root.glob("packages/*/core.lua"):
        for m in _INSTALL.finditer(core.read_text(encoding="utf-8")):
            install_mods.add(m.group(1))
            install_calls += 1

    exports: set = set()
    for mod in install_mods:
        path = _module_path(root, mod)
        if not path.exists():
            continue
        text = path.read_text(encoding="utf-8")
        exports |= _export_names(text)
        for sub in _SUBMOD.findall(text):
            sub_path = _module_path(root, sub)
            if sub_path.exists():
                exports |= _export_names(sub_path.read_text(encoding="utf-8"))
    return {"install_m_calls": install_calls, "ambient_m_exports": len(exports)}


def baseline(root: Path):
    path = root / INVENTORY
    if not path.exists():
        return None
    return json.loads(path.read_text(encoding="utf-8"))


def repository_messages(root: Path):
    if not (root / "libraries" / "devloop").exists():
        return
    cur = counts(root)
    base = baseline(root)
    if base is None:
        yield (
            f"missing baseline {INVENTORY}; create it with {json.dumps(cur)} "
            f"(shrink-only; delete install(M) scaffolds once the ambient M is unconsumed to lower it)"
        )
        return
    for key, value in cur.items():
        prev = base.get(key)
        if prev is not None and value > prev:
            yield (
                f"{key} = {value} (baseline {prev}); this GREW. Do not add install(M) ambient-surface "
                f"exports; migrate departments to make_department(caps) and delete the scaffold. "
                f"Update {INVENTORY} only when the real count drops."
            )


def check(root: Path, violations: list) -> None:
    for message in repository_messages(root):
        violations.append(f"G-DEVLOOP-AMBIENT-SURFACE: {message}")


if __name__ == "__main__":
    root = Path(sys.argv[1]) if len(sys.argv) > 1 else Path(".")
    v: list = []
    check(root, v)
    print("current:", counts(root), "baseline:", baseline(root))
    for m in v:
        print("VIOLATION:", m)
    sys.exit(1 if v else 0)
