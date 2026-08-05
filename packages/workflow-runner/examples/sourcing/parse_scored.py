"""Defensive reader for the agentic step's output.

A model asked for JSON will sometimes wrap it in prose, fence it, or explain
itself first. The failure mode this exists to prevent is **silently scoring
everything zero**: that would publish nothing while reporting success, and the
operator would see a green run and an empty table.

So: extract the payload from whatever shape it arrived in, and when it genuinely
cannot be found, FAIL LOUDLY. A failed step names itself in the run record; a
quietly empty one does not.
"""

from __future__ import annotations

import json
import re

# A fenced block, optionally tagged ```json.
_FENCE = re.compile(r"```(?:json)?\s*(.*?)```", re.DOTALL)


class ScoredPayloadError(ValueError):
    """The step produced nothing this reader can treat as a score list."""


def parse(text: str) -> list[dict]:
    """Return the scored entries, or raise ``ScoredPayloadError``.

    Three attempts, cheapest first: the whole text, then each fenced block, then
    the outermost bracketed span. Anything else is a failure — guessing further
    would risk scoring against a fragment.
    """
    for candidate in _candidates(text):
        try:
            data = json.loads(candidate)
        except json.JSONDecodeError:
            continue
        entries = _as_entries(data)
        if entries is not None:
            return entries
    raise ScoredPayloadError(
        "no JSON array of scored entries found in the step's output; "
        "refusing to treat that as zero scores"
    )


def _candidates(text: str):
    yield text
    for block in _FENCE.findall(text):
        yield block
    # Last resort: the outermost [...] span, for output that is prose with a bare
    # array embedded in it.
    start, end = text.find("["), text.rfind("]")
    if start != -1 and end > start:
        yield text[start : end + 1]


def _as_entries(data: object) -> list[dict] | None:
    """Validate the decoded payload's SHAPE, not just its type.

    An entry missing an id or a score is unusable downstream, and letting it
    through would surface as a confusing failure in the publish step instead of
    here, where the cause is visible.
    """
    if not isinstance(data, list) or not data:
        return None
    entries = []
    for element in data:
        if not isinstance(element, dict):
            return None
        if "id" not in element or "score" not in element:
            return None
        try:
            score = float(element["score"])
        except (TypeError, ValueError):
            return None
        entries.append(
            {
                "id": str(element["id"]),
                "score": score,
                "rationale": str(element.get("rationale", "")),
                "signals": [str(tag) for tag in element.get("signals", [])],
            }
        )
    return entries
