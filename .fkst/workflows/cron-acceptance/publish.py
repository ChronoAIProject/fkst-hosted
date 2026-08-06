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
envelope is stripped -- but only the envelope. Anything still unparseable,
incomplete, duplicated, malformed, or empty fails the step LOUDLY. Acceptance
also requires at least one published row and a new ledger commit, so a run cannot
report success without observable publication evidence.
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
SCORE_KEYS = {"id", "score", "rationale"}


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
    if not candidates:
        raise SystemExit(f"{CANDIDATES} must contain at least one candidate")

    by_id: dict[str, dict] = {}
    for index, entry in enumerate(candidates):
        if not isinstance(entry, dict):
            raise SystemExit(f"{CANDIDATES} entry {index} must be a JSON object")
        entry_id = entry.get("id")
        if not isinstance(entry_id, str) or not entry_id:
            raise SystemExit(
                f"{CANDIDATES} entry {index} must have a non-empty string id"
            )
        if not isinstance(entry.get("title"), str):
            raise SystemExit(
                f"{CANDIDATES} entry {entry_id!r} must have a string title"
            )
        if entry_id in by_id:
            raise SystemExit(f"{CANDIDATES} contains duplicate id {entry_id!r}")
        by_id[entry_id] = entry

    scores = parse_scored(SCORED.read_text(encoding="utf-8"))
    validate_scores(by_id, scores)
    return by_id, scores


def validate_scores(by_id: dict[str, dict], scores: list[dict]) -> None:
    """Require one well-formed integer score for every collected candidate."""
    if not scores:
        raise SystemExit(f"{SCORED} must contain one score for every candidate")
    if len(scores) != len(by_id):
        raise SystemExit(
            f"{SCORED} contains {len(scores)} score(s) for {len(by_id)} candidate(s)"
        )

    seen: set[str] = set()
    for index, scored in enumerate(scores):
        if not isinstance(scored, dict):
            raise SystemExit(f"{SCORED} entry {index} must be a JSON object")
        if set(scored) != SCORE_KEYS:
            raise SystemExit(
                f"{SCORED} entry {index} must contain exactly id, score, and rationale"
            )

        entry_id = scored["id"]
        if not isinstance(entry_id, str) or entry_id not in by_id:
            raise SystemExit(f"{SCORED} entry {index} has an unknown id")
        if entry_id in seen:
            raise SystemExit(f"{SCORED} contains duplicate id {entry_id!r}")
        seen.add(entry_id)

        score = scored["score"]
        if isinstance(score, bool) or not isinstance(score, int) or not 0 <= score <= 10:
            raise SystemExit(
                f"{SCORED} entry {entry_id!r} must have an integer score from 0 to 10"
            )
        rationale = scored["rationale"]
        if not isinstance(rationale, str) or not rationale.strip() or len(rationale) > 120:
            raise SystemExit(
                f"{SCORED} entry {entry_id!r} must have a non-empty rationale "
                "of at most 120 characters"
            )

    if set(by_id) != seen:
        raise SystemExit(f"{SCORED} is missing score(s) for collected candidate ids")


def accepted_rows(
    by_id: dict[str, dict], scores: list[dict], minimum: int
) -> dict[str, dict]:
    """The validated rows clearing `minimum`."""
    rows: dict[str, dict] = {}
    for scored in scores:
        entry_id = scored["id"]
        raw_score = scored["score"]
        if raw_score < minimum:
            continue
        rows[entry_id] = {
            "id": entry_id,
            "title": by_id[entry_id]["title"],
            "score": raw_score,
            "rationale": scored["rationale"],
        }
    return rows


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--min-score", required=True, help="inclusive acceptance threshold")
    args = parser.parse_args()

    try:
        minimum = int(args.min_score)
    except ValueError:
        LOG.error("--min-score must be an integer, got %r", args.min_score)
        return 2
    if not 0 <= minimum <= 10:
        LOG.error("--min-score must be between 0 and 10, got %d", minimum)
        return 2

    by_id, scores = load_entries()
    rows = accepted_rows(by_id, scores, minimum)
    if not rows:
        LOG.error(
            "acceptance requires at least one scored candidate at or above min_score=%d",
            minimum,
        )
        return 1

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
    if not changed:
        LOG.error("acceptance requires a new ledger commit proving publication")
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
