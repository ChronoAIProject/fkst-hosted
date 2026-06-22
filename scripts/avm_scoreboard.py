"""Aggregate autonomy ledger facts into L0-L4 AVM scoreboard rows."""

from __future__ import annotations

from typing import Any


TASK_LEVELS = ("L0", "L1", "L2", "L3", "L4", "unclassified")


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


def avm_facts(data: Any) -> list[dict[str, Any]]:
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
        facts.append(raw)
    return facts


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


def aggregate_avm_scoreboard(data: Any) -> list[dict[str, Any]]:
    buckets = {level: empty_bucket(level) for level in TASK_LEVELS}
    for fact in avm_facts(data):
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
        elif revert_state in {"pass", "fail"}:
            bucket["false_consensus_denominator"] += 1
            if revert_state == "fail":
                bucket["false_consensus_numerator"] += 1

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
