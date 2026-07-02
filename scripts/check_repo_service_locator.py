#!/usr/bin/env python3
"""G-DEVLOOP-SERVICE-LOCATOR: shrink-only ratchet driving the ambient-M service-locator to zero.

The composed `core` (M) is a package-side god-table: `core.lua` builds it via
`require("devloop.X").install(M)` and 36 departments do `local core = require("core")` then read
`core.X`. That is the service-locator anti-pattern the DI refactor retires (see
docs/superpowers/specs/2026-07-02-di-refactor-retire-ambient-m-design.md). This ratchet counts the
READ SIDE of the debt so it can only shrink:

- `require("core")` in department files (departments/**), and
- `core.<member>` reads in department files.

It is the FIRST of two ratchets. The SECOND (G-DEVLOOP-AMBIENT-SURFACE) counts the install(M)
implementation surface, so that moving reads core.X -> caps.X without deleting the ambient surface
cannot fake success (the exact failure a prior audit caught). Migrated departments define
`make_department(caps)` and read narrow injected caps instead of the ambient core.

Shrink-only against migration/service-locator.inventory. Deterministic, read-only.
"""
import json
import re
import sys
from pathlib import Path

INVENTORY = "migration/service-locator.inventory"

_REQUIRE_CORE = re.compile(r'require\(\s*["\']core["\']\s*\)')
_CORE_MEMBER = re.compile(r'\bcore\.[A-Za-z_][A-Za-z0-9_]*')


def _department_files(root: Path):
    for lua in root.glob("packages/*/departments/**/*.lua"):
        rel = lua.as_posix()
        if "/tests/" in rel or rel.endswith("_test.lua"):
            continue
        yield lua


def counts(root: Path) -> dict:
    require_core = 0
    member_reads = 0
    for lua in _department_files(root):
        text = lua.read_text(encoding="utf-8")
        require_core += len(_REQUIRE_CORE.findall(text))
        member_reads += len(_CORE_MEMBER.findall(text))
    return {"department_core_requires": require_core, "department_core_member_reads": member_reads}


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
            f"(shrink-only; migrate departments to make_department(caps) DI to lower it toward zero)"
        )
        return
    for key, value in cur.items():
        prev = base.get(key)
        if prev is not None and value > prev:
            yield (
                f"{key} = {value} (baseline {prev}); this GREW. Departments must not add ambient "
                f"`require(\"core\")` / `core.X` reads; migrate to make_department(caps) narrow "
                f"capabilities. Update {INVENTORY} only when the real count drops."
            )


def check(root: Path, violations: list) -> None:
    for message in repository_messages(root):
        violations.append(f"G-DEVLOOP-SERVICE-LOCATOR: {message}")


if __name__ == "__main__":
    root = Path(sys.argv[1]) if len(sys.argv) > 1 else Path(".")
    v: list = []
    check(root, v)
    print("current:", counts(root), "baseline:", baseline(root))
    for m in v:
        print("VIOLATION:", m)
    sys.exit(1 if v else 0)
