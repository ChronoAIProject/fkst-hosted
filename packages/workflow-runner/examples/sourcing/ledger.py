"""The cross-run de-duplication ledger.

This file is what makes a recurring workflow stateless from the control plane's
point of view: cross-run state is a committed file in the TARGET repository, not
a store the deployment has to own, back up, or migrate.

Two properties it must have, both enforced here rather than left to each script:

* **Bounded growth.** Only the most recent ``RETAIN`` ids are kept. An unbounded
  ledger becomes a file nobody can review and eventually a slow clone.
* **Idempotence.** ``extend`` is a set union that preserves recency order, so
  re-running the same slot adds nothing and a partially-completed publish is
  safely retryable — the ids that landed are already here, the ones that did not
  are retried on the next run.
"""

from __future__ import annotations

import json
from pathlib import Path

# Sized so a daily workflow retains roughly a year of ids. Raise it deliberately
# if a workload's entries recur on a longer cycle than that.
RETAIN = 5000


def load(path: str | Path) -> list[str]:
    """Read the ledger, treating a missing or unreadable one as empty.

    A missing ledger is the FIRST run and must not fail. A corrupt one is
    reported and treated as empty rather than crashing the run: re-publishing an
    entry is recoverable, whereas a workflow that can never run again because one
    commit landed badly is not.
    """
    file = Path(path)
    if not file.exists():
        return []
    try:
        data = json.loads(file.read_text(encoding="utf-8"))
    except (json.JSONDecodeError, OSError) as error:
        print(f"ledger: unreadable ({error}); treating as empty", flush=True)
        return []
    if not isinstance(data, list):
        print("ledger: not a list; treating as empty", flush=True)
        return []
    return [str(entry) for entry in data]


def extend(existing: list[str], accepted: list[str]) -> list[str]:
    """Union `accepted` into `existing`, newest last, bounded to ``RETAIN``."""
    seen = set(existing)
    merged = list(existing)
    for entry_id in accepted:
        if entry_id not in seen:
            seen.add(entry_id)
            merged.append(entry_id)
    return merged[-RETAIN:]


def save(path: str | Path, ids: list[str]) -> None:
    """Write the ledger deterministically.

    Sorted keys and a trailing newline keep the committed diff to the entries
    that actually changed, so a reviewer can see what one run published.
    """
    file = Path(path)
    file.parent.mkdir(parents=True, exist_ok=True)
    file.write_text(json.dumps(ids, indent=2) + "\n", encoding="utf-8")
