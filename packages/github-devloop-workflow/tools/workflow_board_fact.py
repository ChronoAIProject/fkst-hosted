#!/usr/bin/env python3
"""Project trusted github-devloop-workflow markers into one board fact."""

from __future__ import annotations

import argparse
import json
import re
import sys
from typing import Any


MAX_ORIGIN_PROPOSAL_ID_BYTES = 200
MAX_WORKFLOW_ID_BYTES = 128
MAX_PLAN_DIGEST_BYTES = 64
MAX_SLOT_ID_BYTES = 128
MAX_CHILD_ISSUE_BYTES = 30
MAX_TERMINAL_REASON_CODE_BYTES = 128

MATERIALIZATION_STATE_RANK = {
    "pending": 1,
    "generated": 2,
    "created": 3,
}
TERMINAL_STATES = {"done", "blocked", "error"}

MARKER_RE = re.compile(r"<!--\s*fkst:github-devloop-workflow:(blueprint|materialization|terminal):v1\b(.*?)-->")
ATTR_RE = re.compile(r'([A-Za-z_][A-Za-z0-9_]*)="([^"]*)"')


def parser() -> argparse.ArgumentParser:
    result = argparse.ArgumentParser(description=__doc__)
    result.add_argument("--origin", required=True, help="Origin proposal id.")
    result.add_argument("--bot-login", required=True, help="Trusted workflow marker author login.")
    return result


def safe_attr(value: Any, limit: int, *, allow_empty: bool = False) -> str | None:
    if not isinstance(value, str):
        return None
    if value == "":
        return "" if allow_empty else None
    if len(value.encode("utf-8")) > limit:
        return None
    if any(ord(ch) < 32 for ch in value):
        return None
    if any(ch in value for ch in '"<>'):
        return None
    if re.search(r"\s", value):
        return None
    return value


def parse_attrs(raw: str) -> dict[str, str]:
    return {match.group(1): match.group(2) for match in ATTR_RE.finditer(raw)}


def login_for(comment: dict[str, Any]) -> str:
    for key in ("user", "author"):
        raw = comment.get(key)
        if isinstance(raw, dict) and isinstance(raw.get("login"), str):
            return raw["login"]
    return ""


def read_comments() -> tuple[list[Any] | None, str | None]:
    text = sys.stdin.read()
    decoder = json.JSONDecoder()
    comments: list[Any] = []
    index = 0
    try:
        while index < len(text):
            while index < len(text) and text[index].isspace():
                index += 1
            if index >= len(text):
                break
            page, index = decoder.raw_decode(text, index)
            if isinstance(page, list):
                comments.extend(page)
            else:
                return None, "expected comments json array"
    except json.JSONDecodeError as exc:
        return None, f"invalid comments json: {exc}"
    return comments, None


def trusted_comment_bodies(comments: list[Any], bot_login: str) -> list[str]:
    bodies = []
    for comment in comments:
        if not isinstance(comment, dict):
            continue
        if login_for(comment) != bot_login:
            continue
        body = comment.get("body")
        if isinstance(body, str):
            bodies.append(body)
    return bodies


def blueprint_fact(attrs: dict[str, str], origin: str) -> dict[str, str] | None:
    marker_origin = safe_attr(attrs.get("origin"), MAX_ORIGIN_PROPOSAL_ID_BYTES)
    workflow = safe_attr(attrs.get("workflow"), MAX_WORKFLOW_ID_BYTES)
    digest = safe_attr(attrs.get("digest"), MAX_PLAN_DIGEST_BYTES)
    if marker_origin != origin or workflow is None or digest is None:
        return None
    return {"workflow": workflow, "digest": digest}


def materialization_fact(attrs: dict[str, str], origin: str, seq: int) -> dict[str, Any] | None:
    marker_origin = safe_attr(attrs.get("origin"), MAX_ORIGIN_PROPOSAL_ID_BYTES)
    slot = safe_attr(attrs.get("slot"), MAX_SLOT_ID_BYTES)
    state = safe_attr(attrs.get("state"), MAX_SLOT_ID_BYTES)
    child_issue = safe_attr(attrs.get("child_issue"), MAX_CHILD_ISSUE_BYTES, allow_empty=True)
    if marker_origin != origin or slot is None or state not in MATERIALIZATION_STATE_RANK or child_issue is None:
        return None
    return {
        "slot": slot,
        "state": state,
        "child_issue": child_issue or None,
        "seq": seq,
    }


def terminal_fact(attrs: dict[str, str], origin: str, seq: int) -> dict[str, Any] | None:
    marker_origin = safe_attr(attrs.get("origin"), MAX_ORIGIN_PROPOSAL_ID_BYTES)
    state = safe_attr(attrs.get("state"), MAX_SLOT_ID_BYTES)
    reason_code = safe_attr(attrs.get("reason_code"), MAX_TERMINAL_REASON_CODE_BYTES)
    if marker_origin != origin or state not in TERMINAL_STATES or reason_code is None:
        return None
    return {"state": state, "reason_code": reason_code, "seq": seq}


def collect_facts(bodies: list[str], origin: str) -> dict[str, Any]:
    facts: dict[str, Any] = {
        "blueprint": None,
        "materializations": [],
        "terminal": None,
    }
    seq = 0
    for body in bodies:
        for match in MARKER_RE.finditer(body):
            seq += 1
            kind = match.group(1)
            attrs = parse_attrs(match.group(2))
            if kind == "blueprint":
                fact = blueprint_fact(attrs, origin)
                if fact is not None:
                    facts["blueprint"] = fact
            elif kind == "materialization":
                fact = materialization_fact(attrs, origin, seq)
                if fact is not None:
                    facts["materializations"].append(fact)
            elif kind == "terminal":
                fact = terminal_fact(attrs, origin, seq)
                if fact is not None:
                    facts["terminal"] = fact
    return facts


def latest_materialization_by_slot(facts: list[dict[str, Any]]) -> dict[str, dict[str, Any]]:
    by_slot: dict[str, dict[str, Any]] = {}
    for fact in facts:
        rank = MATERIALIZATION_STATE_RANK.get(str(fact.get("state")))
        slot = fact.get("slot")
        if rank is None or not isinstance(slot, str):
            continue
        current = by_slot.get(slot)
        current_rank = MATERIALIZATION_STATE_RANK.get(str(current.get("state"))) if current else None
        if current is None or current_rank is None or rank > current_rank or (
            rank == current_rank and int(fact.get("seq") or 0) >= int(current.get("seq") or 0)
        ):
            by_slot[slot] = fact
    return by_slot


def board_fact(facts: dict[str, Any]) -> tuple[str, str] | None:
    blueprint = facts.get("blueprint")
    terminal = facts.get("terminal")
    materializations = facts.get("materializations")
    if not isinstance(blueprint, dict):
        return None

    workflow = str(blueprint.get("workflow") or "unknown")
    if isinstance(terminal, dict):
        state = str(terminal.get("state") or "unknown")
        reason_code = str(terminal.get("reason_code") or "unknown")
        return "workflow", f"parked(workflow:{workflow} {state}({reason_code}))"

    if isinstance(materializations, list):
        latest_by_slot = latest_materialization_by_slot(materializations)
        if latest_by_slot:
            latest = max(latest_by_slot.values(), key=lambda row: int(row.get("seq") or 0))
            slot = str(latest.get("slot") or "unknown")
            child_issue = latest.get("child_issue")
            if latest.get("state") == "created" and child_issue:
                return "workflow", f"tracking(workflow:{workflow} running(step {slot} -> #{child_issue}))"
            return "workflow", f"tracking(workflow:{workflow} running(step {slot}))"

    return "workflow", f"tracking(workflow:{workflow})"


def main(argv: list[str] | None = None) -> int:
    args = parser().parse_args(argv)
    comments, err = read_comments()
    if comments is None:
        print(f"workflow-board-fact: {err}", file=sys.stderr)
        return 2

    bodies = trusted_comment_bodies(comments, args.bot_login)
    fact = board_fact(collect_facts(bodies, args.origin))
    if fact is None:
        return 1
    print(f"{fact[0]}\t{fact[1]}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
