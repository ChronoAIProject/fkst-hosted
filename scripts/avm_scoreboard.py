"""Aggregate autonomy ledger facts into L0-L4 AVM scoreboard rows."""

from __future__ import annotations

import re
from typing import Any


TASK_LEVELS = ("L0", "L1", "L2", "L3", "L4", "unclassified")
NO_REVERT_REOPEN_WINDOW_SECONDS = 7 * 24 * 60 * 60
NO_REVERT_REOPEN_SCAN_SCHEMA = "github-devloop.no-revert-reopen-scan.v1"
REQUIRED_GATE_NAMES = (
    "human_touch",
    "pre_merge_ci",
    "evidence_manifest",
    "post_merge_probe",
    "no_revert_reopen",
    "cost_budget",
)


def list_from_any(value: Any) -> list[Any]:
    if value is None:
        return []
    if isinstance(value, list):
        return value
    if isinstance(value, dict):
        return list(value.values())
    return []


def number_value(value: Any) -> float | None:
    if isinstance(value, bool) or value is None:
        return None
    try:
        parsed = float(value)
    except (TypeError, ValueError):
        return None
    if parsed < 0:
        return None
    return parsed


def int_value(value: Any) -> int:
    try:
        return int(value)
    except (TypeError, ValueError):
        return 0


def optional_int_value(value: Any) -> int | None:
    parsed = number_value(value)
    if parsed is None or parsed % 1 != 0:
        return None
    return int(parsed)


def task_level(value: Any) -> str:
    text = str(value or "").strip().upper()
    if text in TASK_LEVELS[:-1]:
        return text
    return "unclassified"


def gate_state(value: Any) -> str | None:
    text = str(value or "").strip().lower()
    if text in {"pass", "passed", "true", "green", "success"}:
        return "pass"
    if text in {"fail", "failed", "false", "red", "failure", "invalid_self_attested"}:
        return "fail"
    if text in {"pending", "unknown"}:
        return "pending"
    return None


def first_number(raw: dict[str, Any], keys: tuple[str, ...]) -> float | None:
    for key in keys:
        parsed = number_value(raw.get(key))
        if parsed is not None:
            return parsed
    return None


def first_int(raw: dict[str, Any], keys: tuple[str, ...]) -> int | None:
    for key in keys:
        parsed = optional_int_value(raw.get(key))
        if parsed is not None:
            return parsed
    return None


def nested_gate(raw: dict[str, Any], *names: str) -> str | None:
    for name in names:
        state = gate_state(raw.get(name))
        if state is not None:
            return state
    gates = raw.get("gates")
    if isinstance(gates, dict):
        for name in names:
            state = gate_state(gates.get(name))
            if state is not None:
                return state
    return None


def raw_entity_records(data: Any) -> list[Any]:
    if isinstance(data, list):
        return data
    if isinstance(data, dict):
        for key in ("entities", "entity_timeline", "entity_timelines", "timelines"):
            raw_entities = list_from_any(data.get(key))
            if raw_entities:
                return raw_entities
    return []


def avm_fact_shape(raw: dict[str, Any]) -> bool:
    schema = str(raw.get("schema") or "").lower()
    if "autonomy" in schema or "avm" in schema:
        return True
    return any(
        key in raw
        for key in (
            "valid_autonomous_merge",
            "avm_rate_numerator",
            "avm_rate_denominator",
            "attempt_projection",
            "task_class",
            "task_level",
            "risk_tier",
            "false_consensus",
            "false_consensus_rate_numerator",
        )
    )


def normalize_text(value: Any) -> str:
    return " ".join(str(value or "").lower().split())


def parse_pr_number(value: Any) -> int | None:
    try:
        parsed = int(value)
    except (TypeError, ValueError):
        return None
    if parsed < 1:
        return None
    return parsed


def title_or_body_reverts_pr(pr: dict[str, Any], target_number: int) -> bool:
    text = normalize_text(f"{pr.get('title') or ''}\n{pr.get('body') or ''}")
    if "revert" not in text:
        return False
    target = str(target_number)
    return re.search(rf"(?:#|pull/|pull request |pr ){re.escape(target)}(?!\d)", text) is not None


def timestamp_order(value: Any) -> float | None:
    text = str(value or "").strip()
    if not text:
        return None
    if text.endswith("Z"):
        text = text[:-1] + "+00:00"
    try:
        from datetime import datetime

        return datetime.fromisoformat(text).timestamp()
    except ValueError:
        return None


def timestamp_from_first(*values: Any) -> float | None:
    for value in values:
        parsed = timestamp_order(value)
        if parsed is not None:
            return parsed
    return None


def no_revert_merge_seconds(fact: dict[str, Any]) -> float | None:
    return timestamp_from_first(fact.get("merged_at"), fact.get("mergedAt"), fact.get("comment_created_at"))


def evidence_seconds(raw: dict[str, Any]) -> float | None:
    return timestamp_from_first(
        raw.get("merged_at"),
        raw.get("mergedAt"),
        raw.get("committed_at"),
        raw.get("committedAt"),
        raw.get("authored_at"),
        raw.get("authoredAt"),
        raw.get("pushed_at"),
        raw.get("pushedAt"),
        raw.get("reopened_at"),
        raw.get("reopenedAt"),
        raw.get("updated_at"),
        raw.get("updatedAt"),
        raw.get("closed_at"),
        raw.get("closedAt"),
    )


def evidence_within_no_revert_window(fact: dict[str, Any], raw: dict[str, Any]) -> bool:
    merged = no_revert_merge_seconds(fact)
    evidence = evidence_seconds(raw)
    if merged is None or evidence is None:
        return True
    return merged <= evidence <= merged + NO_REVERT_REOPEN_WINDOW_SECONDS


def pr_records(data: Any) -> list[dict[str, Any]]:
    records: list[dict[str, Any]] = []

    def add(raw: Any) -> None:
        if not isinstance(raw, dict):
            return
        number = parse_pr_number(raw.get("number") or raw.get("pr_number"))
        if number is None:
            return
        row = dict(raw)
        row["number"] = number
        if "merged_at" not in row and "mergedAt" in row:
            row["merged_at"] = row.get("mergedAt")
        records.append(row)

    if isinstance(data, dict):
        for key in ("recent_merged_prs", "merged_prs", "pull_requests", "prs"):
            for raw in list_from_any(data.get(key)):
                add(raw)
    for entity in raw_entity_records(data):
        if not isinstance(entity, dict):
            continue
        add(entity.get("pr"))
        if entity.get("kind") == "pr" or "pr_number" in entity:
            add(entity)
    return records


def issue_reopened(entity: dict[str, Any]) -> bool:
    issue = entity.get("parent_issue") if isinstance(entity.get("parent_issue"), dict) else entity.get("issue")
    if isinstance(issue, dict):
        if str(issue.get("state_reason") or issue.get("stateReason") or "").upper() == "REOPENED":
            return True
        if issue.get("reopened") is True:
            return True
    if str(entity.get("state_reason") or entity.get("stateReason") or "").upper() == "REOPENED":
        return True
    if entity.get("reopened") is True:
        return True
    return entity.get("issue_reopened") is True


def revert_commit_records(fact: dict[str, Any], data: Any) -> list[dict[str, Any]]:
    records: list[dict[str, Any]] = []
    scan = fact.get("no_revert_reopen_scan")
    if isinstance(scan, dict):
        for raw in list_from_any(scan.get("revert_commits")):
            if isinstance(raw, dict):
                records.append(raw)
    if isinstance(data, dict):
        scan = data.get("no_revert_reopen_scan")
        if isinstance(scan, dict):
            for raw in list_from_any(scan.get("revert_commits")):
                if isinstance(raw, dict):
                    records.append(raw)
        for key in ("revert_commits", "recent_revert_commits"):
            for raw in list_from_any(data.get(key)):
                if isinstance(raw, dict):
                    records.append(raw)
    return records


def commit_reverts_pr(commit: dict[str, Any], target_number: int) -> bool:
    explicit = parse_pr_number(
        commit.get("reverted_pr")
        or commit.get("reverted_pr_number")
        or commit.get("target_pr")
        or commit.get("target_pr_number")
    )
    if explicit is not None:
        return explicit == target_number
    return title_or_body_reverts_pr(
        {
            "title": commit.get("message_head") or commit.get("subject") or commit.get("title"),
            "body": commit.get("message_body") or commit.get("body") or commit.get("message"),
        },
        target_number,
    )


def commit_identity(commit: dict[str, Any]) -> str:
    return str(commit.get("sha") or commit.get("oid") or commit.get("commit_sha") or commit.get("revert_commit") or commit.get("id") or "")


def detect_false_consensus(fact: dict[str, Any], data: Any) -> list[dict[str, Any]]:
    pr_number = parse_pr_number(fact.get("pr_number") or fact.get("pr"))
    if pr_number is None:
        return []
    pairs: list[dict[str, Any]] = []
    seen: set[tuple[Any, Any, Any]] = set()

    def add(pair: dict[str, Any]) -> None:
        key = (pair.get("reverted_pr"), pair.get("revert_pr") or pair.get("issue_number") or pair.get("revert_commit"), pair.get("evidence"))
        if key in seen:
            return
        seen.add(key)
        pairs.append(pair)

    for pr in pr_records(data):
        number = parse_pr_number(pr.get("number"))
        if number is None or number == pr_number:
            continue
        if title_or_body_reverts_pr(pr, pr_number) and evidence_within_no_revert_window(fact, pr):
            add({"reverted_pr": pr_number, "revert_pr": number, "evidence": "explicit-revert-pr"})
    for commit in revert_commit_records(fact, data):
        if commit_reverts_pr(commit, pr_number) and evidence_within_no_revert_window(fact, commit):
            add({"reverted_pr": pr_number, "revert_commit": commit_identity(commit), "evidence": "revert-commit"})
    for entity in raw_entity_records(data):
        if not isinstance(entity, dict):
            continue
        if parse_pr_number(entity.get("pr_number")) == pr_number and issue_reopened(entity) and evidence_within_no_revert_window(
            fact,
            entity.get("parent_issue") if isinstance(entity.get("parent_issue"), dict) else entity.get("issue") if isinstance(entity.get("issue"), dict) else entity,
        ):
            add({"reverted_pr": pr_number, "issue_number": parse_pr_number(entity.get("issue_number")), "evidence": "issue-reopened"})
    issue_number = parse_pr_number(fact.get("issue_number") or fact.get("issue"))
    for issue in list_from_any(data.get("recent_merged_issues") if isinstance(data, dict) else None):
        if not isinstance(issue, dict) or issue_number is None:
            continue
        if parse_pr_number(issue.get("number") or issue.get("issue_number")) == issue_number and issue_reopened(issue) and evidence_within_no_revert_window(fact, issue):
            add({"reverted_pr": pr_number, "issue_number": issue_number, "evidence": "issue-reopened"})
    return pairs


def scanned_pr_numbers(data: Any) -> set[int]:
    numbers: set[int] = set()
    if not isinstance(data, dict):
        return numbers
    for raw in list_from_any(data.get("recent_merged_prs")):
        if not isinstance(raw, dict):
            continue
        number = parse_pr_number(raw.get("number") or raw.get("pr_number"))
        if number is not None:
            numbers.add(number)
    return numbers


def scanned_issue_numbers(data: Any) -> set[int]:
    numbers: set[int] = set()
    if not isinstance(data, dict):
        return numbers
    for raw in list_from_any(data.get("recent_merged_issues")):
        if not isinstance(raw, dict):
            continue
        number = parse_pr_number(raw.get("number") or raw.get("issue_number"))
        if number is not None:
            numbers.add(number)
    for entity in raw_entity_records(data):
        if not isinstance(entity, dict):
            continue
        number = parse_pr_number(entity.get("issue_number"))
        if number is not None:
            numbers.add(number)
    return numbers


def valid_autonomous_merge(gates: dict[str, Any]) -> str:
    has_pending = False
    for name in REQUIRED_GATE_NAMES:
        state = gate_state(gates.get(name))
        if state == "fail":
            return "false"
        if state != "pass":
            has_pending = True
    return "pending" if has_pending else "true"


def no_revert_sources_complete(fact: dict[str, Any], data: Any) -> bool:
    if not isinstance(data, dict):
        return False
    pr_number = parse_pr_number(fact.get("pr_number") or fact.get("pr"))
    issue_number = parse_pr_number(fact.get("issue_number") or fact.get("issue"))
    if pr_number is None or issue_number is None:
        return False
    return pr_number in scanned_pr_numbers(data) and issue_number in scanned_issue_numbers(data)


def truthy(value: Any) -> bool:
    return value is True or str(value or "").strip().lower() == "true"


def scan_timestamp(scan: dict[str, Any], *keys: str) -> float | None:
    for key in keys:
        parsed = timestamp_order(scan.get(key))
        if parsed is not None:
            return parsed
    return None


def no_revert_full_window_scan_covers(fact: dict[str, Any], data: Any) -> bool:
    scan = fact.get("no_revert_reopen_scan")
    if not isinstance(scan, dict) and isinstance(data, dict):
        scan = data.get("no_revert_reopen_scan")
    if not isinstance(scan, dict) or scan.get("schema") != NO_REVERT_REOPEN_SCAN_SCHEMA:
        return False
    if not truthy(scan.get("pr_reverts_complete")) or not truthy(scan.get("revert_commits_complete")) or not truthy(scan.get("issue_reopens_complete")):
        return False
    merged = no_revert_merge_seconds(fact)
    since = scan_timestamp(scan, "since", "since_at", "since_seconds")
    until = scan_timestamp(scan, "until", "until_at", "until_seconds")
    if merged is None or since is None or until is None:
        return False
    return since <= merged and until >= merged + NO_REVERT_REOPEN_WINDOW_SECONDS


def no_revert_reopen_gate(fact: dict[str, Any], data: Any, now: Any = None) -> str:
    if detect_false_consensus(fact, data):
        return "fail"
    if not no_revert_sources_complete(fact, data):
        return "pending"
    merged = no_revert_merge_seconds(fact)
    now_seconds = timestamp_order(now)
    if now_seconds is None and isinstance(data, dict):
        now_seconds = timestamp_order(data.get("now") or data.get("generated_at") or data.get("cached_at"))
    if merged is None or now_seconds is None:
        return "pending"
    if now_seconds < merged + NO_REVERT_REOPEN_WINDOW_SECONDS:
        return "pending"
    return "pass" if no_revert_full_window_scan_covers(fact, data) else "pending"


def apply_no_revert_reopen_gate(fact: dict[str, Any], data: Any, now: Any = None) -> dict[str, Any]:
    gate = no_revert_reopen_gate(fact, data, now)
    decorated = dict(fact)
    gates = dict(decorated.get("gates") or {})
    gates["no_revert_reopen"] = gate
    decorated["gates"] = gates
    decorated["valid_autonomous_merge"] = valid_autonomous_merge(gates)
    return decorated


def decorate_false_consensus(fact: dict[str, Any], data: Any) -> dict[str, Any]:
    if "false_consensus" in fact or "false_consensus_rate_numerator" in fact:
        return fact
    pairs = detect_false_consensus(fact, data)
    scanned = scanned_pr_numbers(data)
    if not pairs and parse_pr_number(fact.get("pr_number") or fact.get("pr")) not in scanned:
        return fact
    decorated = dict(fact)
    decorated["false_consensus"] = bool(pairs)
    if pairs:
        decorated["false_consensus_pairs"] = pairs
        gates = dict(decorated.get("gates") or {})
        gates["no_revert_reopen"] = "fail"
        decorated["gates"] = gates
    return decorated


def raw_avm_sources(data: Any) -> list[dict[str, Any]]:
    sources: list[dict[str, Any]] = []

    def add_source(raw: Any) -> None:
        if not isinstance(raw, dict):
            return
        nested = raw.get("autonomy_result")
        if isinstance(nested, dict):
            fact = dict(nested)
            for key in ("proposal_id", "repo", "issue_number", "pr_number", "version", "head_sha"):
                if key not in fact and key in raw:
                    fact[key] = raw[key]
            sources.append(fact)
            return
        payload = raw.get("payload")
        if isinstance(payload, dict):
            add_source(payload)
        if avm_fact_shape(raw):
            sources.append(raw)

    if isinstance(data, dict):
        for key in (
            "avm_facts",
            "autonomy_facts",
            "autonomy_results",
            "autonomy_ledger",
            "competence_facts",
            "avm_scoreboard_facts",
        ):
            for raw in list_from_any(data.get(key)):
                add_source(raw)

    for entity in raw_entity_records(data):
        add_source(entity)
        if not isinstance(entity, dict):
            continue
        add_source(entity.get("latest_event"))
        for event in list_from_any(entity.get("events") or entity.get("timeline") or entity.get("event_timeline")):
            add_source(event)
    return sources


def avm_fact_identity(fact: dict[str, Any]) -> str | None:
    for key in ("merge_id", "attempt_id", "id"):
        value = fact.get(key)
        if value not in (None, ""):
            return f"{key}:{value}"
    parts = [fact.get(key) for key in ("proposal_id", "pr_number", "version", "head_sha")]
    present = [str(part) for part in parts if part not in (None, "")]
    if len(present) >= 2:
        return "merge:" + "|".join(present)
    return None


def avm_facts(data: Any, now: Any = None) -> list[dict[str, Any]]:
    facts: list[dict[str, Any]] = []
    seen: set[str] = set()
    for raw in raw_avm_sources(data):
        if not avm_fact_shape(raw):
            continue
        identity = avm_fact_identity(raw)
        if identity is not None:
            if identity in seen:
                continue
            seen.add(identity)
        facts.append(decorate_false_consensus(apply_no_revert_reopen_gate(raw, data, now), data))
    return facts


def false_consensus_pairs(data: Any, now: Any = None) -> list[dict[str, Any]]:
    pairs: list[dict[str, Any]] = []
    seen: set[tuple[Any, Any, Any]] = set()
    for fact in avm_facts(data, now):
        for pair in list_from_any(fact.get("false_consensus_pairs")):
            if not isinstance(pair, dict):
                continue
            key = (pair.get("reverted_pr"), pair.get("revert_pr") or pair.get("issue_number") or pair.get("revert_commit"), pair.get("evidence"))
            if key in seen:
                continue
            seen.add(key)
            pairs.append(pair)
    pairs.sort(
        key=lambda row: (
            int_value(row.get("reverted_pr")),
            int_value(row.get("revert_pr") or row.get("issue_number")),
            str(row.get("revert_commit") or ""),
            str(row.get("evidence") or ""),
        )
    )
    return pairs


def empty_bucket(level: str) -> dict[str, Any]:
    return {
        "level": level,
        "merges": 0,
        "avm_numerator": 0,
        "avm_denominator": 0,
        "cost_total": 0.0,
        "cost_missing": False,
        "rounds": [],
        "revert_numerator": 0,
        "revert_denominator": 0,
        "false_consensus_numerator": 0,
        "false_consensus_denominator": 0,
    }


def avm_rate_parts(fact: dict[str, Any]) -> tuple[int, int]:
    numerator = first_int(fact, ("avm_rate_numerator", "valid_merges"))
    denominator = first_int(fact, ("avm_rate_denominator", "total_attempts"))
    if numerator is not None and denominator is not None:
        return max(0, numerator), max(0, denominator)
    projection = fact.get("attempt_projection")
    if isinstance(projection, dict):
        numerator = first_int(projection, ("valid_merges", "avm_rate_numerator"))
        denominator = first_int(projection, ("total_attempts", "avm_rate_denominator"))
        if numerator is not None and denominator is not None:
            return max(0, numerator), max(0, denominator)
    valid = str(fact.get("valid_autonomous_merge") or "").strip().lower()
    if valid in {"true", "false", "pending", "invalid_self_attested"}:
        return (1 if valid == "true" else 0), 1
    return 0, 0


def avm_cost(fact: dict[str, Any]) -> float | None:
    return first_number(fact, ("cost", "total_cost", "cost_units", "codex_calls", "token_cost"))


def explicit_false_consensus_parts(fact: dict[str, Any]) -> tuple[int, int] | None:
    numerator = first_int(fact, ("false_consensus_rate_numerator", "false_consensus_numerator"))
    denominator = first_int(fact, ("false_consensus_rate_denominator", "false_consensus_denominator"))
    if numerator is not None and denominator is not None:
        return max(0, numerator), max(0, denominator)
    value = fact.get("false_consensus")
    if isinstance(value, bool):
        return (1 if value else 0), 1
    if isinstance(value, str) and value.strip().lower() in {"true", "false"}:
        return (1 if value.strip().lower() == "true" else 0), 1
    return None


def aggregate_avm_scoreboard(data: Any, now: Any = None) -> list[dict[str, Any]]:
    buckets = {level: empty_bucket(level) for level in TASK_LEVELS}
    for fact in avm_facts(data, now):
        bucket = buckets[task_level(fact.get("task_level") or fact.get("task_class") or fact.get("risk_tier"))]
        bucket["merges"] += 1

        numerator, denominator = avm_rate_parts(fact)
        bucket["avm_numerator"] += numerator
        bucket["avm_denominator"] += denominator

        cost = avm_cost(fact)
        if cost is None:
            bucket["cost_missing"] = True
        else:
            bucket["cost_total"] += cost

        rounds = first_int(fact, ("rounds", "median_rounds", "merge_rounds"))
        if rounds is not None:
            bucket["rounds"].append(rounds)

        revert_state = nested_gate(fact, "no_revert_reopen", "gate_no_revert_reopen", "revert", "reopened")
        if revert_state in {"pass", "fail"}:
            bucket["revert_denominator"] += 1
            if revert_state == "fail":
                bucket["revert_numerator"] += 1

        false_parts = explicit_false_consensus_parts(fact)
        if false_parts is not None:
            bucket["false_consensus_numerator"] += false_parts[0]
            bucket["false_consensus_denominator"] += false_parts[1]

    return [buckets[level] for level in TASK_LEVELS]


def format_decimal(value: float) -> str:
    text = f"{value:.2f}".rstrip("0").rstrip(".")
    return text or "0"


def format_rate(numerator: int, denominator: int) -> str:
    if denominator <= 0:
        return "n/a"
    pct = (numerator / denominator) * 100
    return f"{numerator}/{denominator} ({format_decimal(pct)}%)"


def format_median(values: list[int]) -> str:
    if not values:
        return "n/a"
    ordered = sorted(values)
    mid = len(ordered) // 2
    if len(ordered) % 2 == 1:
        return str(ordered[mid])
    return format_decimal((ordered[mid - 1] + ordered[mid]) / 2)


def format_cost_per_avm(bucket: dict[str, Any]) -> str:
    if bucket["merges"] == 0:
        return "n/a"
    if bucket["cost_missing"]:
        return "unknown"
    avms = int_value(bucket.get("avm_numerator"))
    if avms <= 0:
        return "n/a"
    return format_decimal(float(bucket["cost_total"]) / avms)


def render_avm_bucket(bucket: dict[str, Any]) -> str:
    return (
        f"- {bucket['level']} merges={bucket['merges']} "
        f"AVM-rate={format_rate(int_value(bucket['avm_numerator']), int_value(bucket['avm_denominator']))} "
        f"cost-per-AVM={format_cost_per_avm(bucket)} "
        f"revert-rate={format_rate(int_value(bucket['revert_numerator']), int_value(bucket['revert_denominator']))} "
        f"median-rounds={format_median(bucket['rounds'])} "
        f"false-consensus-rate={format_rate(int_value(bucket['false_consensus_numerator']), int_value(bucket['false_consensus_denominator']))}"
    )
