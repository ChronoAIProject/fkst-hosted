#!/usr/bin/env python3
"""Free-form saga handler migration ratchet."""

from __future__ import annotations

import re
from pathlib import Path

import ratchet_base


ALLOWLIST = "migration/saga-handler.allowlist"
SAGA_REQUIRE_RE = re.compile(r"""\brequire\s*(?:\(\s*)?["']workflow\.saga["']""")
SAGA_DEPARTMENT_RE = re.compile(r"\.\s*department\s*[({]")
FREE_FORM_PIPELINE_RE = re.compile(r"(?m)^\s*(?:function\s+pipeline\s*\(|pipeline\s*=\s*function\b)")


def is_saga_handler_source(source: str, strip_lua_comments_and_strings) -> bool:
    return SAGA_REQUIRE_RE.search(source) is not None and SAGA_DEPARTMENT_RE.search(strip_lua_comments_and_strings(source)) is not None


def ratchet_violations(
    sources: dict[str, str],
    allowlist: set[str],
    strip_lua_comments_and_strings,
    base_allowlist: set[str] | None = None,
) -> list[str]:
    violations: list[str] = []
    for path, source in sorted(sources.items()):
        stripped = strip_lua_comments_and_strings(source)
        saga_shaped = is_saga_handler_source(source, strip_lua_comments_and_strings)
        if saga_shaped and path in allowlist:
            violations.append(f"G10: {path} saga-shaped department remains on saga-handler allowlist; remove it")
        if saga_shaped and FREE_FORM_PIPELINE_RE.search(stripped) is not None:
            violations.append(f"G10: {path} saga-shaped department still defines free-form top-level pipeline")
        if not saga_shaped and path not in allowlist:
            violations.append(f"G10: {path} free-form department not on saga-handler allowlist; migrate to workflow.saga.department or (only for pre-existing) keep listed")
    for path in sorted(allowlist - set(sources)):
        violations.append(f"G10: {path} listed in saga-handler allowlist but does not exist")
    if base_allowlist is not None:
        violations.extend(f"G10: {path} grows saga-handler allowlist relative to dev; migrate instead" for path in sorted(allowlist - base_allowlist))
    return violations


def allowlist_at_dev_base(root: Path) -> tuple[str, set[str] | None]:
    try:
        status, shown = ratchet_base.file_at_base(root, ALLOWLIST)
        if status != "present":
            return status, None
        assert shown is not None
        return "present", {line.strip() for line in shown.splitlines() if line.strip() and not line.lstrip().startswith("#")}
    except Exception:
        return "unresolved", None
