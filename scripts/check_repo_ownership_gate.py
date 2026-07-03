#!/usr/bin/env python3
"""Ownership claim guard for PR review issue claims."""

from __future__ import annotations

import re
from pathlib import Path


CLAIMS_PATH = Path("libraries/devloop/claims.lua")
GATE_RE = re.compile(r"(?ms)^\s*function\s+M\s*\.\s*verify_pr_review_issue_claim\s*\([^)]*\).*?(?=^\s*function\s+M\s*\.|\Z)")


def defaulting_bot_login_lines(text: str, strip_lua_comments_and_strings) -> list[int]:
    stripped = strip_lua_comments_and_strings(text)
    gate = GATE_RE.search(stripped)
    if gate is None:
        return []
    lines: list[int] = []
    for match in re.finditer(r"\btrusted_bot_login\s*\(", stripped[gate.start() : gate.end()]):
        lines.append(text.count("\n", 0, gate.start() + match.start()) + 1)
    return lines
