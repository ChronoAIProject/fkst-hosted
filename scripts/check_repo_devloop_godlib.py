"""G-DEVLOOP-GODLIB: shrink-only coupling ratchet for the libraries/devloop god-PATTERN.

The harmful "god-lib" shape in libraries/devloop is NOT its file count (a prior audit
judged devloop a cohesive single-product kernel) but the shared mutable-M coupling:
modules `install(M)` symbols onto one ambient table, packages assemble that table via
`require("devloop.*").install(M)`, and devloop exports a `devloop.*` wildcard. This
ratchet measures that coupling and forbids growth; dissolution = driving the counts to 0
by converting modules to typed `require(...)`-returns-a-table form.

Metrics (libraries/devloop unless noted):
  install_defs        - `function X.install(M)` / `.install = function(M)` definitions
  m_writes            - `M.<symbol> =` assignments + `function M.<symbol>(` definitions
  package_core_installs - `require("devloop.*").install(M)` call sites in packages/*/core.lua
  wildcard_exports    - `devloop.*` wildcard entries in libraries/devloop/fkst.toml

The committed baseline lives in migration/devloop-godlib.inventory (JSON). CI fails if any
current count EXCEEDS its baseline (growth forbidden); recording progress = lowering the
baseline numbers in that file. This is a self-contained shrink-only ratchet (the inventory
file is the allowlist), mirroring the simplest repository_messages(root) check shape.
"""
from __future__ import annotations

import json
import re
from pathlib import Path

INVENTORY = "migration/devloop-godlib.inventory"
DEVLOOP = "libraries/devloop"
FKST_TOML = "libraries/devloop/fkst.toml"

_INSTALL_DEF = re.compile(r"(?:function\s+[A-Za-z_][\w.]*\.install\s*\(\s*M\s*\)|\.install\s*=\s*function\s*\(\s*M\s*\))")
_M_ASSIGN = re.compile(r"(?<![\w.])M\.[A-Za-z_]\w*\s*=")
_M_FUNC = re.compile(r"function\s+M\.[A-Za-z_]\w*\s*\(")
_PKG_INSTALL = re.compile(r'require\(\s*"devloop\.[A-Za-z_][\w.]*"\s*\)\s*\.install\s*\(\s*M\s*\)')
_WILDCARD = re.compile(r'"devloop\.\*"')


def _lua_files(base: Path):
    return sorted(p for p in base.rglob("*.lua") if p.is_file())


def measure(root: Path) -> dict:
    """Return the current coupling counts. Deterministic, read-only."""
    devloop = root / DEVLOOP
    install_defs = 0
    m_writes = 0
    if devloop.is_dir():
        for path in _lua_files(devloop):
            text = path.read_text(encoding="utf-8", errors="replace")
            install_defs += len(_INSTALL_DEF.findall(text))
            m_writes += len(_M_ASSIGN.findall(text)) + len(_M_FUNC.findall(text))

    package_core_installs = 0
    packages = root / "packages"
    if packages.is_dir():
        for core in sorted(packages.glob("*/core.lua")):
            if core.is_file():
                package_core_installs += len(_PKG_INSTALL.findall(core.read_text(encoding="utf-8", errors="replace")))

    wildcard_exports = 0
    toml = root / FKST_TOML
    if toml.is_file():
        wildcard_exports = len(_WILDCARD.findall(toml.read_text(encoding="utf-8", errors="replace")))

    return {
        "install_defs": install_defs,
        "m_writes": m_writes,
        "package_core_installs": package_core_installs,
        "wildcard_exports": wildcard_exports,
    }


def load_baseline(root: Path) -> dict | None:
    path = root / INVENTORY
    if not path.is_file():
        return None
    try:
        data = json.loads(path.read_text(encoding="utf-8"))
    except (ValueError, OSError):
        return None
    counts = data.get("counts", data)
    return {k: int(counts.get(k, 0)) for k in ("install_defs", "m_writes", "package_core_installs", "wildcard_exports")}


def ratchet_messages(current: dict, baseline: dict | None):
    """Yield a message for every metric that GREW above baseline (shrink-only)."""
    if baseline is None:
        yield (
            "missing baseline " + INVENTORY + "; create it with the current counts "
            "(this is the shrink-only baseline; growth is forbidden, progress lowers it)"
        )
        return
    for key in ("install_defs", "m_writes", "package_core_installs", "wildcard_exports"):
        cur = int(current.get(key, 0))
        base = int(baseline.get(key, 0))
        if cur > base:
            yield (
                f"devloop god-PATTERN coupling grew: {key} {cur} > baseline {base}. "
                f"Adding shared-M install/exports is forbidden; dissolve via typed modules "
                f"(require-returns-a-table), never grow the ambient-M surface."
            )


def repository_messages(root: Path):
    """Entry point mirrored on the other repository_messages(root) checks.

    No-op when there is no libraries/devloop to govern (external/synthetic repos),
    so this ratchet only enforces where the god-PATTERN can actually exist.
    """
    if not (root / DEVLOOP).is_dir():
        return
    yield from ratchet_messages(measure(root), load_baseline(root))


if __name__ == "__main__":  # manual: print current counts vs baseline
    import sys

    r = Path(sys.argv[1]) if len(sys.argv) > 1 else Path(__file__).resolve().parents[1]
    cur = measure(r)
    base = load_baseline(r)
    print("current:", json.dumps(cur))
    print("baseline:", json.dumps(base))
    msgs = list(ratchet_messages(cur, base))
    for m in msgs:
        print("VIOLATION:", m)
    sys.exit(1 if msgs else 0)
