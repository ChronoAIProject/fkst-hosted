#!/usr/bin/env python3
"""Render a local github-devloop board from generic engine observe data."""

from __future__ import annotations

import argparse
import json
import os
import subprocess
import sys
from datetime import datetime, timezone
from pathlib import Path
from typing import Any

from avm_scoreboard import aggregate_avm_scoreboard, false_consensus_pairs, render_avm_bucket


DEFAULT_TTL_SECONDS = 60
DEFAULT_STALL_SECONDS = 30 * 60
MAX_ENTITIES = 40
MAX_QUEUES = 40
ANOMALY_LIMIT = 40
EXPECTED_TRANSIENT_LIMIT = 20


def env_int(name: str, default: int) -> int:
    raw = os.environ.get(name)
    if raw is None or raw == "":
        return default
    try:
        value = int(raw)
    except ValueError:
        raise SystemExit(f"error: {name} must be an integer, got {raw!r}")
    if value < 0:
        raise SystemExit(f"error: {name} must be non-negative")
    return value


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Render a read-only github-devloop board from fkst-framework observe --json.",
    )
    parser.add_argument("--bin", required=True, help="Path to fkst-framework.")
    parser.add_argument("--durable-root", required=True, help="FKST_DURABLE_ROOT to observe.")
    parser.add_argument("--cache", required=True, help="Local board cache JSON path.")
    parser.add_argument("--refresh", action="store_true", help="Bypass the TTL cache and re-read observe data.")
    parser.add_argument("--ttl", type=int, default=env_int("FKST_BOARD_CACHE_TTL_SECONDS", DEFAULT_TTL_SECONDS))
    parser.add_argument("--stall", type=int, default=env_int("FKST_BOARD_STALL_SECONDS", DEFAULT_STALL_SECONDS))
    parser.add_argument("--health", action="store_true", help=argparse.SUPPRESS)
    parser.add_argument("--now", help=argparse.SUPPRESS)
    return parser.parse_args()


def parse_time(value: Any) -> datetime | None:
    if value is None:
        return None
    if isinstance(value, (int, float)):
        timestamp = float(value)
        if timestamp > 10_000_000_000:
            timestamp = timestamp / 1000.0
        return datetime.fromtimestamp(timestamp, tz=timezone.utc)
    text = str(value).strip()
    if not text:
        return None
    if text.isdigit():
        return parse_time(int(text))
    if text.endswith("Z"):
        text = text[:-1] + "+00:00"
    try:
        parsed = datetime.fromisoformat(text)
    except ValueError:
        return None
    if parsed.tzinfo is None:
        parsed = parsed.replace(tzinfo=timezone.utc)
    return parsed.astimezone(timezone.utc)


def iso(dt: datetime) -> str:
    return dt.astimezone(timezone.utc).replace(microsecond=0).isoformat().replace("+00:00", "Z")


def human_duration(seconds: float | int | None) -> str:
    if seconds is None:
        return "unknown"
    total = max(0, int(seconds))
    days, rem = divmod(total, 86400)
    hours, rem = divmod(rem, 3600)
    minutes, secs = divmod(rem, 60)
    if days:
        return f"{days}d{hours}h"
    if hours:
        return f"{hours}h{minutes}m"
    if minutes:
        return f"{minutes}m{secs}s"
    return f"{secs}s"


def first_string(obj: dict[str, Any], keys: tuple[str, ...], default: str = "-") -> str:
    for key in keys:
        value = obj.get(key)
        if value is None:
            continue
        if isinstance(value, dict):
            nested = value.get("ref") or value.get("id") or value.get("kind")
            if nested is not None:
                return str(nested)
        elif isinstance(value, list):
            continue
        else:
            text = str(value)
            if text:
                return text
    return default


def list_from_any(value: Any) -> list[Any]:
    if value is None:
        return []
    if isinstance(value, list):
        return value
    if isinstance(value, dict):
        return list(value.values())
    return []


def bool_value(value: Any) -> bool:
    if value is True:
        return True
    if isinstance(value, str):
        return value.strip().lower() == "true"
    return False


def int_value(value: Any) -> int:
    try:
        return int(value)
    except (TypeError, ValueError):
        return 0


def event_timestamp(event: dict[str, Any] | None) -> datetime | None:
    if not isinstance(event, dict):
        return None
    for key in ("ts", "time", "at", "observed_at", "updated_at", "created_at", "observed_at_ms", "event_ts"):
        parsed = parse_time(event.get(key))
        if parsed is not None:
            return parsed
    return None


def event_sort_key(event: dict[str, Any]) -> float:
    parsed = event_timestamp(event)
    if parsed is None:
        return -1.0
    return parsed.timestamp()


def event_name(event: dict[str, Any] | None) -> str:
    if not isinstance(event, dict):
        return "-"
    queue = first_string(event, ("queue", "event_queue"), "")
    kind = first_string(event, ("event", "kind", "type", "name", "department"), "")
    if queue and kind and queue != kind:
        return f"{queue}/{kind}"
    if queue:
        return queue
    if kind:
        return kind
    return "-"


def raw_entity_records(data: Any) -> list[Any]:
    if isinstance(data, list):
        return data
    elif isinstance(data, dict):
        for key in ("entities", "entity_timeline", "entity_timelines", "timelines"):
            raw_entities = list_from_any(data.get(key))
            if raw_entities:
                return raw_entities
    return []


def latest_entity_events(data: Any, now: datetime) -> list[dict[str, Any]]:
    records: list[dict[str, Any]] = []
    for raw in raw_entity_records(data):
        if not isinstance(raw, dict):
            continue
        events = list_from_any(raw.get("events") or raw.get("timeline") or raw.get("event_timeline"))
        latest_event = raw.get("latest_event") if isinstance(raw.get("latest_event"), dict) else None
        if latest_event is None and events:
            latest_event = max((ev for ev in events if isinstance(ev, dict)), key=event_sort_key, default=None)
        if latest_event is None:
            latest_event = raw

        event_time = event_timestamp(latest_event)
        dwell_seconds = None if event_time is None else (now - event_time).total_seconds()
        records.append(
            {
                "entity": first_string(raw, ("entity", "entity_id", "id", "source_ref", "ref", "key", "proposal")),
                "latest": event_name(latest_event),
                "latest_at": iso(event_time) if event_time else "-",
                "dwell_seconds": dwell_seconds,
                "dwell": human_duration(dwell_seconds),
                "event": latest_event,
                "entity_terminal": bool_value(raw.get("terminal")),
            }
        )

    records.sort(key=lambda row: (row["dwell_seconds"] is None, -(row["dwell_seconds"] or 0), row["entity"]))
    return records


def all_entity_events(data: Any, now: datetime) -> list[dict[str, Any]]:
    records: list[dict[str, Any]] = []
    for raw in raw_entity_records(data):
        if not isinstance(raw, dict):
            continue
        entity = first_string(raw, ("entity", "entity_id", "id", "source_ref", "ref", "key", "proposal"))
        events = list_from_any(raw.get("events") or raw.get("timeline") or raw.get("event_timeline"))
        if not events:
            latest_event = raw.get("latest_event") if isinstance(raw.get("latest_event"), dict) else raw
            events = [latest_event]
        for event in events:
            if not isinstance(event, dict):
                continue
            event_time = event_timestamp(event)
            dwell_seconds = None if event_time is None else (now - event_time).total_seconds()
            records.append(
                {
                    "entity": entity,
                    "latest": event_name(event),
                    "latest_at": iso(event_time) if event_time else "-",
                    "dwell_seconds": dwell_seconds,
                    "dwell": human_duration(dwell_seconds),
                    "event": event,
                    "entity_terminal": bool_value(raw.get("terminal")),
                }
            )
    records.sort(key=lambda row: (row["dwell_seconds"] is None, -(row["dwell_seconds"] or 0), row["entity"]))
    return records


def entity_records(data: Any, now: datetime) -> list[dict[str, Any]]:
    records = []
    for row in latest_entity_events(data, now):
        records.append(
            {
                "entity": row["entity"],
                "latest": row["latest"],
                "latest_at": row["latest_at"],
                "dwell_seconds": row["dwell_seconds"],
                "dwell": row["dwell"],
                "entity_terminal": row["entity_terminal"],
            }
        )
    return records


def metric(raw: dict[str, Any], keys: tuple[str, ...]) -> str:
    for key in keys:
        value = raw.get(key)
        if value is None:
            continue
        if isinstance(value, list):
            return str(len(value))
        return str(value)
    return "0"


def queue_records(data: Any) -> list[dict[str, str]]:
    if not isinstance(data, dict):
        return []
    raw_queues = []
    for key in ("queues", "queue_state", "queue_states"):
        raw_queues = list_from_any(data.get(key))
        if raw_queues:
            break
    rows = []
    for raw in raw_queues:
        if not isinstance(raw, dict):
            continue
        rows.append(
            {
                "queue": first_string(raw, ("queue", "name", "id")),
                "ready": metric(raw, ("ready", "pending", "due", "available")),
                "leased": metric(raw, ("leased", "inflight", "running", "active")),
                "retry": metric(raw, ("retry", "retries", "delayed", "backoff")),
                "dlq": metric(raw, ("dlq", "dead", "dead_letters", "dead_letter")),
            }
        )
    rows.sort(key=lambda row: row["queue"])
    return rows


def dlq_count(data: Any) -> int | None:
    if not isinstance(data, dict):
        return None
    for key in ("dlq", "dead_letters", "dead_letter"):
        value = data.get(key)
        if isinstance(value, list):
            return len(value)
        if isinstance(value, dict):
            return len(value)
        if isinstance(value, int):
            return value
    return None


def summary_fields(event: dict[str, Any]) -> str:
    parts = []
    for key in ("outcome", "disposition", "terminal", "error_class", "fingerprint", "tag"):
        if key in event and event.get(key) is not None:
            parts.append(f"{key}={event.get(key)}")
    source_ref = first_string(event, ("source_ref",), "")
    if source_ref:
        parts.append(f"source_ref={source_ref}")
    return " ".join(parts)


def expected_transient(row: dict[str, Any]) -> bool:
    event = row.get("event")
    if not isinstance(event, dict):
        return False
    return (
        event.get("disposition") == "expected-transient"
        or event.get("outcome") == "retry-pending"
        or event.get("outcome") == "skip-foreign"
        or event.get("outcome") == "deadline-defer"
        or event.get("error_class") == "retry-pending"
        or event.get("error_class") == "marker-lag"
    )


def failure_fact_records(data: Any) -> list[dict[str, Any]]:
    if not isinstance(data, dict):
        return []
    for key in ("failure_facts", "failures", "error_facts"):
        facts = list_from_any(data.get(key))
        if facts:
            return [fact for fact in facts if isinstance(fact, dict)]
    return []


def fact_source_ref_kind(fact: dict[str, Any]) -> str:
    source_ref = fact.get("source_ref")
    if isinstance(source_ref, dict):
        return str(source_ref.get("kind") or "")
    return ""


def fact_queue(fact: dict[str, Any]) -> str:
    return first_string(fact, ("origin_queue", "queue", "event_queue"))


def fact_dept(fact: dict[str, Any]) -> str:
    return first_string(fact, ("origin_dept", "dept", "department", "dead_dept"), fact_queue(fact))


def cron_failure_fact(fact: dict[str, Any]) -> bool:
    queue = fact_queue(fact)
    return queue.endswith("_tick") or fact_source_ref_kind(fact) == "cron"


def infra_liveness_anomalies(data: Any) -> tuple[list[dict[str, Any]], set[str]]:
    grouped: dict[str, dict[str, Any]] = {}
    suppressed_queues: set[str] = set()
    for fact in failure_fact_records(data):
        if not cron_failure_fact(fact):
            continue
        dept = fact_dept(fact)
        queue = fact_queue(fact)
        key = dept if dept != "-" else queue
        row = grouped.setdefault(
            key,
            {
                "type": "infra-stall",
                "queue": queue,
                "details": f"infra-stall:{key} " + summary_fields(fact),
                "count": 1,
                "observed_count": 0,
            },
        )
        row["observed_count"] = int_value(row.get("observed_count")) + 1
        if queue != "-":
            suppressed_queues.add(queue)
    rows = []
    for row in grouped.values():
        if int_value(row.get("observed_count")) > 1:
            row["details"] = f"{row['details']} observed_count={row['observed_count']}"
            rows.append(row)
    return rows, suppressed_queues


def failure_fact_anomalies(data: Any, suppressed_queues: set[str] | None = None) -> list[dict[str, Any]]:
    rows = []
    suppressed = suppressed_queues or set()
    for fact in failure_fact_records(data):
        if not (bool_value(fact.get("terminal")) or fact.get("disposition") == "terminal"):
            continue
        if fact_queue(fact) in suppressed:
            continue
        rows.append(
            {
                "type": "terminal-failure",
                "queue": fact_queue(fact),
                "details": summary_fields(fact),
                "count": 1,
            }
        )
    return rows


def failure_fact_expected_transients(data: Any) -> list[dict[str, Any]]:
    rows = []
    for fact in failure_fact_records(data):
        if bool_value(fact.get("terminal")) or fact.get("disposition") == "terminal":
            continue
        if not expected_transient({"event": fact}):
            continue
        details = summary_fields({**fact, "disposition": "expected-transient"})
        rows.append(
            {
                "entity": first_string(fact, ("origin_dept", "origin_queue", "queue")),
                "latest": first_string(fact, ("origin_queue", "queue", "event_queue")),
                "latest_at": "-",
                "dwell": "unknown",
                "details": details,
            }
        )
    return rows


def dead_letter_anomalies(data: Any, suppressed_queues: set[str] | None = None) -> list[dict[str, Any]]:
    if not isinstance(data, dict):
        return []
    suppressed = suppressed_queues or set()
    for key in ("dlq", "dead_letters", "dead_letter"):
        if key not in data:
            continue
        value = data.get(key)
        if isinstance(value, int):
            return [{"type": "queue-dlq", "queue": "-", "details": f"count={value}", "count": value}]
        rows = []
        for raw in list_from_any(value):
            if isinstance(raw, dict):
                queue = first_string(raw, ("queue", "event_queue", "name"))
                if queue in suppressed:
                    continue
                rows.append(
                    {
                        "type": "queue-dlq",
                        "queue": queue,
                        "details": summary_fields(raw),
                        "count": 1,
                    }
                )
        return rows

    rows = []
    for queue in queue_records(data):
        count = int_value(queue.get("dlq"))
        if count > 0 and queue["queue"] not in suppressed:
            rows.append({"type": "queue-dlq", "queue": queue["queue"], "details": f"count={count}", "count": count})
    return rows


def anomaly_records(data: Any, now: datetime, stall_seconds: int) -> list[dict[str, Any]]:
    anomalies: list[dict[str, Any]] = []
    for row in latest_entity_events(data, now):
        event = row.get("event")
        if not isinstance(event, dict):
            continue
        common = {
            "entity": row["entity"],
            "latest": row["latest"],
            "latest_at": row["latest_at"],
            "dwell": row["dwell"],
            "details": summary_fields(event),
            "count": 1,
        }
        if bool_value(event.get("safety_violation")) or event.get("disposition") == "safety-violation":
            anomalies.append({"type": "safety-violation", **common})
            continue
        if bool_value(event.get("terminal")) or event.get("disposition") == "terminal" or event.get("tag") == "DEAD_LETTER":
            anomalies.append({"type": "terminal-failure", **common})
            continue
        if expected_transient(row):
            continue
        if row["entity_terminal"] is not True and row["dwell_seconds"] is not None and row["dwell_seconds"] > stall_seconds:
            anomalies.append({"type": "stalled-entity", **common})

    infra, suppressed_queues = infra_liveness_anomalies(data)
    anomalies.extend(infra)
    anomalies.extend(failure_fact_anomalies(data, suppressed_queues))
    anomalies.extend(dead_letter_anomalies(data, suppressed_queues))
    return anomalies


def expected_transient_records(data: Any, now: datetime) -> list[dict[str, Any]]:
    rows = []
    for row in all_entity_events(data, now):
        if expected_transient(row):
            event = row["event"]
            rows.append(
                {
                    "entity": row["entity"],
                    "latest": row["latest"],
                    "latest_at": row["latest_at"],
                    "dwell": row["dwell"],
                    "details": summary_fields(event),
                }
            )
    rows.extend(failure_fact_expected_transients(data))
    return rows


def anomaly_count(anomalies: list[dict[str, Any]]) -> int:
    return sum(max(1, int_value(row.get("count"))) for row in anomalies)


def health_line(anomalies: list[dict[str, Any]]) -> str:
    count = anomaly_count(anomalies)
    if count == 0:
        return "HEALTHY"
    return f"{count} ANOMALIES NEEDING ATTENTION"


def render_anomaly(row: dict[str, Any]) -> str:
    if row["type"] in {"queue-dlq", "infra-stall"}:
        details = f" {row['details']}" if row.get("details") else ""
        return f"- type={row['type']} queue={row.get('queue', '-')}{details}"
    if "entity" not in row:
        details = f" {row['details']}" if row.get("details") else ""
        return f"- type={row['type']} queue={row.get('queue', '-')}{details}"
    details = f" {row['details']}" if row.get("details") else ""
    return (
        f"- type={row['type']} entity={row['entity']} latest={row['latest']} "
        f"at={row['latest_at']} dwell={row['dwell']}{details}"
    )


def render_false_consensus_pair(pair: dict[str, Any]) -> str | None:
    reverted = int_value(pair.get("reverted_pr"))
    if reverted <= 0:
        return None
    evidence = str(pair.get("evidence") or "explicit-revert-pr")
    revert_pr = int_value(pair.get("revert_pr"))
    if revert_pr > 0:
        return f"- PR #{reverted} reverted-by PR #{revert_pr} evidence={evidence}"
    issue_number = int_value(pair.get("issue_number"))
    if issue_number > 0:
        return f"- PR #{reverted} issue=#{issue_number} evidence={evidence}"
    revert_commit = str(pair.get("revert_commit") or "")
    if revert_commit:
        return f"- PR #{reverted} reverted-by commit {revert_commit} evidence={evidence}"
    return None


def render(
    data: Any,
    *,
    source: str,
    cached_at: datetime,
    now: datetime,
    durable_root: str,
    stall_seconds: int,
    health_only: bool = False,
) -> str:
    entity_events = latest_entity_events(data, now)
    entities = entity_records(data, now)
    queues = queue_records(data)
    stalls = [
        row
        for row in entity_events
        if row["entity_terminal"] is not True
        and row["dwell_seconds"] is not None
        and row["dwell_seconds"] > stall_seconds
        and not expected_transient(row)
    ]
    dead = dlq_count(data)
    anomalies = anomaly_records(data, now, stall_seconds)
    transients = expected_transient_records(data, now)
    avm_scoreboard = aggregate_avm_scoreboard(data, iso(now))
    churn_pairs = false_consensus_pairs(data, iso(now))
    if health_only:
        return health_line(anomalies) + "\n"

    lines = [
        health_line(anomalies),
        "fkst-dev local board",
        f"source={source} cached_at={iso(cached_at)} durable_root={durable_root}",
        "",
        "Anomalies needing attention",
    ]
    if anomalies:
        for row in anomalies[:ANOMALY_LIMIT]:
            lines.append(render_anomaly(row))
        if len(anomalies) > ANOMALY_LIMIT:
            lines.append(f"- ... {len(anomalies) - ANOMALY_LIMIT} more")
    else:
        lines.append("- none")

    lines.extend(["", "Expected transients"])
    if transients:
        for row in transients[:EXPECTED_TRANSIENT_LIMIT]:
            details = f" {row['details']}" if row["details"] else ""
            lines.append(f"- {row['entity']} latest={row['latest']} at={row['latest_at']} dwell={row['dwell']}{details}")
        if len(transients) > EXPECTED_TRANSIENT_LIMIT:
            lines.append(f"- ... {len(transients) - EXPECTED_TRANSIENT_LIMIT} more")
    else:
        lines.append("- none")

    lines.extend(["", "AVM scoreboard by task level"])
    for bucket in avm_scoreboard:
        lines.append(render_avm_bucket(bucket))

    lines.extend(["", "False consensus churn"])
    if churn_pairs:
        shown = 0
        for pair in churn_pairs[:MAX_ENTITIES]:
            rendered = render_false_consensus_pair(pair)
            if rendered is None:
                continue
            lines.append(rendered)
            shown += 1
        if len(churn_pairs) > shown:
            lines.append(f"- ... {len(churn_pairs) - shown} more")
    else:
        lines.append("- none")

    lines.extend([
        "",
        "Entities",
    ])
    if entities:
        for row in entities[:MAX_ENTITIES]:
            lines.append(f"- {row['entity']} latest={row['latest']} at={row['latest_at']} dwell={row['dwell']}")
        if len(entities) > MAX_ENTITIES:
            lines.append(f"- ... {len(entities) - MAX_ENTITIES} more")
    else:
        lines.append("- none")

    lines.extend(["", f"Stall suspects threshold={human_duration(stall_seconds)}"])
    if stalls:
        for row in stalls[:MAX_ENTITIES]:
            lines.append(f"- {row['entity']} latest={row['latest']} dwell={row['dwell']}")
        if len(stalls) > MAX_ENTITIES:
            lines.append(f"- ... {len(stalls) - MAX_ENTITIES} more")
    else:
        lines.append("- none")

    lines.extend(["", "Queues"])
    if queues:
        for row in queues[:MAX_QUEUES]:
            lines.append(
                f"- {row['queue']} ready={row['ready']} leased={row['leased']} retry={row['retry']} dlq={row['dlq']}"
            )
        if len(queues) > MAX_QUEUES:
            lines.append(f"- ... {len(queues) - MAX_QUEUES} more")
    else:
        lines.append("- none")
    if dead is not None:
        lines.append(f"DLQ total={dead}")
    return "\n".join(lines) + "\n"


def read_cache(path: Path, now: datetime, ttl_seconds: int) -> tuple[Any, datetime] | None:
    if not path.exists():
        return None
    try:
        envelope = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError):
        return None
    if not isinstance(envelope, dict) or envelope.get("schema") != "fkst.board-cache.v1":
        return None
    cached_at = parse_time(envelope.get("cached_at"))
    if cached_at is None:
        return None
    age = (now - cached_at).total_seconds()
    if age < 0 or age > ttl_seconds:
        return None
    return envelope.get("observe"), cached_at


def write_cache(path: Path, data: Any, cached_at: datetime) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    tmp = path.with_name(path.name + ".tmp")
    envelope = {"schema": "fkst.board-cache.v1", "cached_at": iso(cached_at), "observe": data}
    tmp.write_text(json.dumps(envelope, sort_keys=True, separators=(",", ":")) + "\n", encoding="utf-8")
    tmp.replace(path)


def fetch_observe(args: argparse.Namespace) -> Any:
    env = os.environ.copy()
    env["FKST_DURABLE_ROOT"] = args.durable_root
    command = [
        args.bin,
        "observe",
        "--durable-root",
        args.durable_root,
        "--json",
    ]
    result = subprocess.run(
        command,
        env=env,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        check=False,
    )
    if result.returncode != 0:
        detail = result.stderr.strip() or result.stdout.strip() or f"exit {result.returncode}"
        raise RuntimeError(
            "fkst-framework observe --json failed; fkst-substrate#81 is required for scripts/run.sh board: "
            + detail
        )
    try:
        return json.loads(result.stdout)
    except json.JSONDecodeError as exc:
        raise RuntimeError(f"fkst-framework observe --json returned invalid JSON: {exc}") from exc


def main() -> int:
    args = parse_args()
    if args.ttl < 0 or args.stall < 0:
        print("error: --ttl and --stall must be non-negative", file=sys.stderr)
        return 2

    now = parse_time(args.now) if args.now else datetime.now(timezone.utc)
    if now is None:
        print(f"error: --now is not a valid timestamp: {args.now}", file=sys.stderr)
        return 2

    cache_path = Path(args.cache)
    cached = None if args.refresh else read_cache(cache_path, now, args.ttl)
    if cached is not None:
        data, cached_at = cached
        print(
            render(
                data,
                source="cache",
                cached_at=cached_at,
                now=now,
                durable_root=args.durable_root,
                stall_seconds=args.stall,
                health_only=args.health,
            ),
            end="",
        )
        return 0

    try:
        data = fetch_observe(args)
        cached_at = now
        write_cache(cache_path, data, cached_at)
    except Exception as exc:
        print(f"error: {exc}", file=sys.stderr)
        return 1

    print(
        render(
            data,
            source="observe",
            cached_at=cached_at,
            now=now,
            durable_root=args.durable_root,
            stall_seconds=args.stall,
            health_only=args.health,
        ),
        end="",
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
