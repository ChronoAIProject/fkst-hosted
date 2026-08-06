"""Step 1 — query the external API, drop known entries, enrich the rest.

The SHAPE, not an instance: `search` and `enrich` are the two places an operator
plugs in their own API. Everything around them — pagination, rate-limit backoff,
per-entry failure tolerance, and the ledger filter — is the part worth copying.

Failure policy, which is the whole point of this file:

* an individual enrichment failing is TOLERATED — one flaky call out of two
  hundred is not a reason to lose the other 199;
* the search itself failing is FATAL — a partial candidate set would look like a
  quiet week rather than an outage, and step 3 would publish that impression.
"""

from __future__ import annotations

import argparse
import json
import os
import sys
import time
import urllib.error
import urllib.parse
import urllib.request

import ledger

# The credential arrives ONLY through the environment, from an environment
# profile. It is never an argument, never in the definition, never in the repo.
API_TOKEN_ENV = "SOURCING_API_TOKEN"

MAX_PAGES = 20
MAX_RETRIES = 5


def request_json(url: str, token: str) -> dict:
    """One authenticated GET, retrying a rate-limit or 5xx with backoff.

    Exponential with a cap: an API that is rate-limiting us wants us to slow
    down, and a tight retry loop is how a scheduled job gets an account banned.
    """
    for attempt in range(MAX_RETRIES):
        request = urllib.request.Request(url, headers={"Authorization": f"Bearer {token}"})
        try:
            with urllib.request.urlopen(request, timeout=30) as response:
                return json.loads(response.read().decode("utf-8"))
        except urllib.error.HTTPError as error:
            if error.code not in (429, 500, 502, 503, 504) or attempt == MAX_RETRIES - 1:
                raise
            # Honour Retry-After when the server states one; otherwise back off.
            delay = int(error.headers.get("Retry-After") or 0) or min(2**attempt, 60)
            print(f"scrape: {error.code}, retrying in {delay}s", flush=True)
            time.sleep(delay)
        except urllib.error.URLError as error:
            if attempt == MAX_RETRIES - 1:
                raise
            print(f"scrape: {error}, retrying", flush=True)
            time.sleep(min(2**attempt, 60))
    raise RuntimeError("unreachable: the retry loop always returns or raises")


def search(role: str, token: str) -> list[dict]:
    """Every result for `role`, following pagination to exhaustion.

    `MAX_PAGES` is a guard, not a limit anyone should hit: an API that keeps
    handing out pages forever would otherwise turn one slot into an unbounded
    run. Hitting it is reported rather than silently truncating.
    """
    entries, page = [], 1
    while page <= MAX_PAGES:
        # <-- Replace this URL with your own API's search endpoint.
        payload = request_json(
            f"https://api.example.invalid/search?q={urllib.parse.quote(role)}&page={page}",
            token,
        )
        batch = payload.get("items") or []
        entries.extend(batch)
        if not batch or not payload.get("has_more"):
            return entries
        page += 1
    print(f"scrape: stopped at the {MAX_PAGES}-page guard; results may be incomplete", flush=True)
    return entries


def enrich(entry: dict, token: str) -> dict:
    """One extra call per entry. Returns the entry unchanged on failure.

    Tolerated deliberately — see the module docstring.
    """
    try:
        # <-- Replace with your own API's detail endpoint.
        detail = request_json(f"https://api.example.invalid/items/{entry['id']}", token)
        return {**entry, **detail}
    except Exception as error:  # noqa: BLE001 - any failure is per-entry, not fatal
        print(f"scrape: enrichment failed for {entry.get('id')}: {error}", flush=True)
        return entry


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--role", required=True)
    parser.add_argument("--ledger", required=True)
    parser.add_argument("--out", required=True)
    args = parser.parse_args()

    token = os.environ.get(API_TOKEN_ENV)
    if not token:
        # Fail closed and name the KEY, never a value: an unauthenticated run
        # would return an empty result set that reads exactly like a quiet week.
        print(
            f"scrape: {API_TOKEN_ENV} is not set; add it to this session's "
            "environment profile",
            file=sys.stderr,
        )
        return 1

    known = set(ledger.load(args.ledger))
    found = search(args.role, token)
    fresh = [entry for entry in found if str(entry.get("id")) not in known]
    print(f"scrape: {len(found)} found, {len(fresh)} new", flush=True)

    enriched = [enrich(entry, token) for entry in fresh]
    with open(args.out, "w", encoding="utf-8") as handle:
        json.dump(enriched, handle, indent=2)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
