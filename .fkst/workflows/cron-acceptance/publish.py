#!/usr/bin/env python3
"""Step 3 of the milestone-24 acceptance workflow: publish and record.

Filters the scored entries by `--min-score`, writes the accepted ones into the
destination table, appends their ids to the cross-run ledger, and commits both
to the ledger branch.

Two properties #5846 asks for by name:

  * Keyed by entry id, so a partial failure of this step is safely retryable --
    re-publishing an id overwrites its row rather than appending a duplicate.
  * Idempotent commit, so re-running the same slot cannot double-publish. That
    lives in `ledger.write`, which skips a commit whose tree is unchanged.

The scored file is parsed DEFENSIVELY. The agentic step is a language model
told to emit bare JSON, and a model that wraps its answer in a fence or adds a
sentence of preamble is producing the right answer in the wrong envelope. So the
envelope is stripped -- but only the envelope. Anything still unparseable fails
the step LOUDLY, because silently scoring an unreadable payload as zero would
publish nothing while reporting success, and a run that succeeds at doing
nothing is worse than one that fails.
"""

from __future__ import annotations

import argparse
import json
import logging
import pathlib
import re
import sys

import ledger

LOG = logging.getLogger("publish")

CANDIDATES = pathlib.Path("candidates.json")
SCORED = pathlib.Path("scored.json")

# ```json ... ``` or ``` ... ```, the two envelopes a model actually produces.
FENCE = re.compile(r"```(?:json)?\s*(.*?)\s*```", re.DOTALL)


def parse_scored(raw: str) -> list[dict]:
    """Decode the agentic step's output, tolerating fences and prose."""
    for candidate in _candidate_payloads(raw):
        try:
            decoded = json.loads(candidate)
        except json.JSONDecodeError:
            continue
        if isinstance(decoded, list):
            return decoded
        # A model that emitted {"entries": [...]} answered correctly in a
        # different shape; accept the single obvious array it contains.
        if isinstance(decoded, dict):
            arrays = [value for value in decoded.values() if isinstance(value, list)]
            if len(arrays) == 1:
                return arrays[0]
    raise SystemExit(
        f"{SCORED} did not contain a JSON array of scores. Refusing to treat an "
        f"unreadable payload as an empty one."
    )


def _candidate_payloads(raw: str):
    """The substrings worth attempting, most-literal first."""
    yield raw
    for match in FENCE.finditer(raw):
        yield match.group(1)
    # Prose on either side of a bare array.
    start, end = raw.find("["), raw.rfind("]")
    if start != -1 and end > start:
        yield raw[start : end + 1]


def load_entries() -> tuple[dict[str, dict], list[dict]]:
    """The candidates this run collected and the scores the model returned."""
    if not CANDIDATES.exists():
        raise SystemExit(f"{CANDIDATES} is missing; step 1 did not produce it")
    if not SCORED.exists():
        raise SystemExit(f"{SCORED} is missing; step 2 did not produce it")
    try:
        candidates = json.loads(CANDIDATES.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise SystemExit(f"{CANDIDATES} is unreadable: {error}") from error
    if not isinstance(candidates, list):
        raise SystemExit(f"{CANDIDATES} must be a JSON array")
    by_id = {entry["id"]: entry for entry in candidates if isinstance(entry, dict) and "id" in entry}
    return by_id, parse_scored(SCORED.read_text(encoding="utf-8"))


def accepted_rows(
    by_id: dict[str, dict], scores: list[dict], minimum: int
) -> tuple[dict[str, dict], int]:
    """The rows clearing `minimum`, plus how many scores were unusable."""
    rows: dict[str, dict] = {}
    unusable = 0
    for scored in scores:
        if not isinstance(scored, dict):
            unusable += 1
            continue
        entry_id = scored.get("id")
        raw_score = scored.get("score")
        if entry_id not in by_id or not isinstance(raw_score, (int, float)):
            # A score for an id this run never collected, or a non-numeric
            # score, is reported rather than silently dropped: it means the
            # agentic step drifted from its instructions.
            unusable += 1
            continue
        if raw_score < minimum:
            continue
        rows[entry_id] = {
            "id": entry_id,
            "title": by_id[entry_id].get("title", ""),
            "score": raw_score,
            "rationale": str(scored.get("rationale", ""))[:200],
        }
    return rows, unusable


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--min-score", required=True, help="inclusive acceptance threshold")
    args = parser.parse_args()

    try:
        minimum = int(args.min_score)
    except ValueError:
        LOG.error("--min-score must be an integer, got %r", args.min_score)
        return 2

    by_id, scores = load_entries()
    if len(scores) != len(by_id):
        LOG.warning(
            "step 2 returned %d score(s) for %d candidate(s); every candidate should appear once",
            len(scores),
            len(by_id),
        )

    rows, unusable = accepted_rows(by_id, scores, minimum)
    if unusable:
        LOG.warning("%d score entr(ies) were unusable and did not publish", unusable)

    try:
        recorded, published = ledger.read()
        published.update(rows)
        # Order-preserving append: previously published ids keep their position,
        # so the sliding window in step 1 stays stable across runs.
        for entry_id in sorted(rows):
            if entry_id not in recorded:
                recorded.append(entry_id)
        changed = ledger.write(
            recorded,
            published,
            f"chore(cron-acceptance): publish {len(rows)} row(s), ledger at {len(recorded)}",
        )
    except ledger.LedgerError as error:
        LOG.error("%s", error)
        return 1

    LOG.info(
        "min_score=%d accepted=%d table_rows=%d ledger=%d committed=%s",
        minimum,
        len(rows),
        len(published),
        len(recorded),
        changed,
    )
    return 0


if __name__ == "__main__":
    logging.basicConfig(level=logging.INFO, format="%(levelname)s %(name)s: %(message)s")
    sys.exit(main())
