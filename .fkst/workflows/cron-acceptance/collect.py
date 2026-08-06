#!/usr/bin/env python3
"""Step 1 of the milestone-24 acceptance workflow: collect candidates.

Deterministic. Emits `candidates.json` for the agentic scoring step, having
first dropped every id the cross-run ledger already records as published.

The ledger read is what makes a run after the seeded first run provably
different, which is the property #5846 asks this step to demonstrate. The
window slides by one id per run and overlaps the previous run by one. Acceptance
therefore requires observable prior state: at least one id must be suppressed
and at least one genuinely new id must be carried forward. A first run seeds the
ledger but cannot, by itself, prove cross-run suppression.

Candidates are generated locally rather than fetched. #5846 draws a content
boundary around the concrete instance -- its API, search terms, and credentials
are operator-supplied and must not enter this repository -- so this step proves
the machinery around the fetch, and the operator swaps the fetch in.
"""

from __future__ import annotations

import argparse
import json
import logging
import pathlib
import re
import sys

import ledger

LOG = logging.getLogger("collect")

CANDIDATES = pathlib.Path("candidates.json")

# One overlapping id per run: enough to prove the ledger suppressed something,
# without suppressing so much that a run has nothing left to score.
OVERLAP = 1

# A slug keeps generated ids inside the character set the run record's `steps`
# attribute can carry, and keeps a hostile `--topic` from reaching a filename.
SLUG = re.compile(r"[^a-z0-9-]+")


def slugify(topic: str) -> str:
    """A filesystem- and marker-safe form of an operator-supplied topic."""
    slug = SLUG.sub("-", topic.strip().lower()).strip("-")
    if not slug:
        raise ValueError(f"topic {topic!r} contains no usable characters")
    return slug[:40]


def build_window(slug: str, published: list[str], count: int) -> list[dict[str, str]]:
    """The ids this run considers, overlapping the previous run by OVERLAP."""
    start = max(0, len(published) - OVERLAP)
    return [
        {"id": f"{slug}-{index:04d}", "title": f"{slug} candidate {index}"}
        for index in range(start, start + count)
    ]


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--topic", required=True, help="operator-supplied subject")
    parser.add_argument("--count", required=True, help="window size for this run")
    args = parser.parse_args()

    try:
        count = int(args.count)
    except ValueError:
        LOG.error("--count must be an integer, got %r", args.count)
        return 2
    if not 1 <= count <= 100:
        LOG.error("--count must be between 1 and 100, got %d", count)
        return 2

    try:
        slug = slugify(args.topic)
    except ValueError as error:
        LOG.error("%s", error)
        return 2

    try:
        recorded, _ = ledger.read()
    except ledger.LedgerError as error:
        LOG.error("%s", error)
        return 1
    published = set(recorded)
    window = build_window(slug, sorted(published), count)
    fresh = [entry for entry in window if entry["id"] not in published]
    suppressed = len(window) - len(fresh)

    if suppressed < OVERLAP:
        LOG.error(
            "acceptance requires a seeded ledger that suppresses at least %d candidate(s); observed %d",
            OVERLAP,
            suppressed,
        )
        return 1
    if not fresh:
        LOG.error("acceptance requires at least one new candidate after ledger suppression")
        return 1

    CANDIDATES.write_text(json.dumps(fresh, indent=2) + "\n", encoding="utf-8")
    LOG.info(
        "topic=%s window=%d suppressed_by_ledger=%d carried_forward=%d -> %s",
        slug,
        len(window),
        suppressed,
        len(fresh),
        CANDIDATES,
    )
    return 0


if __name__ == "__main__":
    logging.basicConfig(level=logging.INFO, format="%(levelname)s %(name)s: %(message)s")
    sys.exit(main())
