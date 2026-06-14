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


DEFAULT_TTL_SECONDS = 60
DEFAULT_STALL_SECONDS = 30 * 60
MAX_ENTITIES = 40
MAX_QUEUES = 40


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
    parser.add_argument("--project-root", required=True, help="Package repository root.")
    parser.add_argument("--durable-root", required=True, help="FKST_DURABLE_ROOT to observe.")
    parser.add_argument("--cache", required=True, help="Local board cache JSON path.")
    parser.add_argument("--refresh", action="store_true", help="Bypass the TTL cache and re-read observe data.")
    parser.add_argument("--ttl", type=int, default=env_int("FKST_BOARD_CACHE_TTL_SECONDS", DEFAULT_TTL_SECONDS))
    parser.add_argument("--stall", type=int, default=env_int("FKST_BOARD_STALL_SECONDS", DEFAULT_STALL_SECONDS))
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


def entity_records(data: Any, now: datetime) -> list[dict[str, Any]]:
    if isinstance(data, list):
        raw_entities = data
    elif isinstance(data, dict):
        raw_entities = []
        for key in ("entities", "entity_timeline", "entity_timelines", "timelines"):
            raw_entities = list_from_any(data.get(key))
            if raw_entities:
                break
    else:
        raw_entities = []

    records: list[dict[str, Any]] = []
    for raw in raw_entities:
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
            }
        )

    records.sort(key=lambda row: (row["dwell_seconds"] is None, -(row["dwell_seconds"] or 0), row["entity"]))
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


def render(data: Any, *, source: str, cached_at: datetime, now: datetime, durable_root: str, stall_seconds: int) -> str:
    entities = entity_records(data, now)
    queues = queue_records(data)
    stalls = [row for row in entities if row["dwell_seconds"] is not None and row["dwell_seconds"] > stall_seconds]
    dead = dlq_count(data)

    lines = [
        "fkst-dev local board",
        f"source={source} cached_at={iso(cached_at)} durable_root={durable_root}",
        "",
        "Entities",
    ]
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
        "--project-root",
        args.project_root,
        "--durable-root",
        args.durable_root,
        "--json",
    ]
    result = subprocess.run(command, env=env, text=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE, check=False)
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
        print(render(data, source="cache", cached_at=cached_at, now=now, durable_root=args.durable_root, stall_seconds=args.stall), end="")
        return 0

    try:
        data = fetch_observe(args)
        cached_at = now
        write_cache(cache_path, data, cached_at)
    except Exception as exc:
        print(f"error: {exc}", file=sys.stderr)
        return 1

    print(render(data, source="observe", cached_at=cached_at, now=now, durable_root=args.durable_root, stall_seconds=args.stall), end="")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
