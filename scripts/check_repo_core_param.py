#!/usr/bin/env python3
"""G-DEVLOOP-CORE-PARAM: shrink-only ratchet for the composed-core/M threaded-parameter coupling.

The DI-refactor audit found the deepest coupling is NOT the department direct reads (the earlier
installer arc drove those from packages/**), but the composed core/M threaded as a CONTEXT
PARAMETER through the devloop library layer: hundreds of `function C.fn(M, ...)` library functions
take the whole composed core, and department call-sites pass `core` into them
(`m_claims.verify(core, ...)`, `parsers_pr.parse(core, ...)`, `config.write_mode(core)`). This is
"god-lib-by-parameter" -- explicit but still the whole god-table. Much of it is vestigial (the M
param is never read) or base-constant-only (→ require("devloop.base")).

This ratchet counts that coupling so the library-layer decoupling can only shrink:
- library functions in libraries/devloop/ taking core/M as the FIRST parameter
  (`function <T>.name(M, ...)` / `function <T>.name(core, ...)`), and
- department call-sites passing core/M as an argument (`ident.method(core[,)]`) in
  packages/*/departments/.

Together with G-DEVLOOP-SERVICE-LOCATOR (dept reads) and G-DEVLOOP-AMBIENT-SURFACE (install(M)
surface), driving all three to zero is the mechanical, un-gameable proof that the ambient-M
god-table is genuinely dissolved. Shrink-only against migration/core-param.inventory.
"""
import json
import re
import sys
from pathlib import Path

INVENTORY = "migration/core-param.inventory"

# function <table>.name(M, ...) or (core, ...) as the first param, in devloop library modules.
_LIB_PARAM = re.compile(
    r'function\s+[A-Za-z_][A-Za-z0-9_]*\.[A-Za-z_][A-Za-z0-9_]*\(\s*(?:M|core)\s*[,)]'
)
# a call-site passing core/M as the first arg: ident.method(core[,)] / (M[,)].
_CALL_ARG = re.compile(
    r'\b[A-Za-z_][A-Za-z0-9_]*\.[A-Za-z_][A-Za-z0-9_]*\(\s*(?:core|M)\s*[,)]'
)


def counts(root: Path) -> dict:
    lib_params = 0
    for lua in root.glob("libraries/devloop/**/*.lua"):
        if lua.name.endswith("_test.lua"):
            continue
        lib_params += len(_LIB_PARAM.findall(lua.read_text(encoding="utf-8")))

    call_args = 0
    for lua in root.glob("packages/*/departments/**/*.lua"):
        rel = lua.as_posix()
        if "/tests/" in rel or rel.endswith("_test.lua"):
            continue
        call_args += len(_CALL_ARG.findall(lua.read_text(encoding="utf-8")))
    return {"library_core_params": lib_params, "dept_core_arg_call_sites": call_args}


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
            f"(shrink-only; decouple devloop library functions from the threaded composed core -- "
            f"drop vestigial M, use require(\"devloop.base\") for base constants, or pass a narrow cap)"
        )
        return
    for key, value in cur.items():
        prev = base.get(key)
        if prev is not None and value > prev:
            yield (
                f"{key} = {value} (baseline {prev}); this GREW. Do not thread the composed core/M "
                f"as a parameter; decouple the library function (drop vestigial M / require the "
                f"module directly / pass a narrow cap). Update {INVENTORY} only when it drops."
            )


def check(root: Path, violations: list) -> None:
    for message in repository_messages(root):
        violations.append(f"G-DEVLOOP-CORE-PARAM: {message}")


if __name__ == "__main__":
    root = Path(sys.argv[1]) if len(sys.argv) > 1 else Path(".")
    v: list = []
    check(root, v)
    print("current:", counts(root), "baseline:", baseline(root))
    for m in v:
        print("VIOLATION:", m)
    sys.exit(1 if v else 0)
