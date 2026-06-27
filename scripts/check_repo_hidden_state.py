#!/usr/bin/env python3
"""Shrink-only ratchet for behavioral hidden-state replay debt."""

from __future__ import annotations

import re
from dataclasses import dataclass
from pathlib import Path

import ratchet_base


ALLOWLIST = "migration/hidden-state.allowlist"
CORE_CONFORMANCE = "packages/github-devloop/core/hidden_state_conformance.lua"
SHARED_CONFORMANCE = "libraries/devloop/hidden_state_conformance.lua"
PACKAGE_INSTALLERS = (
    "packages/github-devloop/core.lua",
    "packages/github-devloop-pr/core.lua",
)
SPAN_AGGREGATORS = (
    "packages/github-devloop/core/span_conformance.lua",
    "packages/github-devloop-pr/core/span_conformance.lua",
)


@dataclass(frozen=True, order=True)
class HiddenStateKey:
    package: str
    row: str
    fact_family: str
    successor: str

    @classmethod
    def parse(cls, line: str) -> "HiddenStateKey":
        parts = line.split("|")
        if len(parts) < 6:
            raise ValueError(f"invalid {ALLOWLIST} line: {line}")
        package, row, fact_family, successor, issue, why = parts[:6]
        if not package or not row or not fact_family or not successor:
            raise ValueError(f"invalid {ALLOWLIST} tuple: {line}")
        if re.fullmatch(r"issue=#?\d+", issue) is None:
            raise ValueError(f"invalid {ALLOWLIST} issue link: {line}")
        if not why.startswith("why=") or why == "why=":
            raise ValueError(f"invalid {ALLOWLIST} WHY: {line}")
        return cls(package, row, fact_family, successor)

    def label(self) -> str:
        return f"{self.package}|{self.row}|{self.fact_family}|{self.successor}"


def load_allowlist(path: Path) -> set[HiddenStateKey]:
    if not path.exists():
        return set()
    entries: set[HiddenStateKey] = set()
    for raw in path.read_text(encoding="utf-8").splitlines():
        stripped = raw.strip()
        if not stripped or stripped.startswith("#"):
            continue
        entries.add(HiddenStateKey.parse(stripped))
    return entries


def allowlist_at_dev_base(root: Path) -> tuple[str, set[HiddenStateKey] | None]:
    try:
        status, shown = ratchet_base.file_at_base(root, ALLOWLIST)
        if status != "present":
            return status, None
        assert shown is not None
        return "present", {
            HiddenStateKey.parse(line.strip())
            for line in shown.splitlines()
            if line.strip() and not line.lstrip().startswith("#")
        }
    except Exception:
        return "unresolved", None


def ratchet_messages(allowlist: set[HiddenStateKey], base_allowlist: set[HiddenStateKey] | None = None) -> list[str]:
    messages: list[str] = []
    if base_allowlist is not None:
        for key in sorted(allowlist - base_allowlist):
            messages.append(f"{key.label()} grows {ALLOWLIST} relative to dev; make the row pass the behavioral poll harness or keep the migration shrinking")
    return messages


def repository_messages(root: Path, allowlist_dir: Path | None = None, enforce_base: bool = True) -> list[str]:
    messages: list[str] = []
    core_path = root / SHARED_CONFORMANCE
    if not core_path.exists():
        messages.append(f"missing {SHARED_CONFORMANCE}; hidden-state conformance must run through package tests")
    else:
        text = core_path.read_text(encoding="utf-8")
        required_tokens = (
            "behavioral_errors",
            "build_fixture",
            "positive poll fixture",
            "negative poll fixture",
            "core.replay_from_table",
            "migration/hidden-state.allowlist",
            "non_durable_advance",
        )
        for token in required_tokens:
            if token not in text:
                messages.append(f"{SHARED_CONFORMANCE} must include behavioral hidden-state harness token {token!r}")
        forbidden_tokens = ("safe_entity_view", "opaque_comments", "synthetic_undeclared_fact_canary", "_hidden_state_capability")
        for token in forbidden_tokens:
            if token in text:
                messages.append(f"{SHARED_CONFORMANCE} must not retain rejected capability hidden-state machinery token {token!r}")
    wrapper_path = root / CORE_CONFORMANCE
    if not wrapper_path.exists():
        messages.append(f"missing {CORE_CONFORMANCE}; github-devloop must expose hidden-state conformance through core")
    elif "devloop.hidden_state_conformance" not in wrapper_path.read_text(encoding="utf-8"):
        messages.append(f"{CORE_CONFORMANCE} must delegate to devloop.hidden_state_conformance")
    for installer in PACKAGE_INSTALLERS:
        path = root / installer
        if not path.exists() or "hidden_state_conformance" not in path.read_text(encoding="utf-8"):
            messages.append(f"{installer} must install behavioral hidden_state_conformance")
    for aggregator in SPAN_AGGREGATORS:
        path = root / aggregator
        text = path.read_text(encoding="utf-8") if path.exists() else ""
        if "hidden_state_conformance_errors" not in text:
            messages.append(f"{aggregator} must include hidden_state_conformance_errors in span conformance")
    replayer_guard = root / "libraries/devloop/replayer_hidden_state.lua"
    if replayer_guard.exists():
        messages.append("libraries/devloop/replayer_hidden_state.lua retains rejected capability hidden-state machinery; delete it")
    dispatch_path = root / "libraries/devloop/replayer.lua"
    if dispatch_path.exists():
        text = dispatch_path.read_text(encoding="utf-8")
        if "replayer_hidden_state" in text or "_replay_advancing_fact_audit" in text:
            messages.append("libraries/devloop/replayer.lua must call replay_from_table with ordinary facts, not hidden-state capability hooks")
    allow_path = root / ALLOWLIST if allowlist_dir is None else allowlist_dir / Path(ALLOWLIST).name
    allowlist = load_allowlist(allow_path)
    base_status, base_allowlist = allowlist_at_dev_base(root) if enforce_base else ("absent", None)
    if base_status == "unresolved":
        messages.append("cannot resolve dev base allowlist to enforce shrink-only ratchet; ensure CI provides the dev ref")
    messages.extend(ratchet_messages(allowlist, base_allowlist))
    return messages


if __name__ == "__main__":
    found = repository_messages(Path.cwd())
    for message in found:
        print(message)
    raise SystemExit(1 if found else 0)
