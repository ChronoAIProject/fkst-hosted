"""Step 3 — filter by score, publish through the credential broker, commit the ledger.

The property that matters here is **retry safety**. A partial failure is normal:
the network drops, the broker rate-limits, the pod is evicted. So every published
row is keyed by entry id, and the ledger is appended for exactly the ids that
landed — which means a re-run publishes only what did not.

The ledger commit is last and is idempotent: nothing to commit is a success, not
a failure, so a run that published nothing new does not fail on `git commit`.
"""

from __future__ import annotations

import argparse
import json
import os
import subprocess
import sys
import urllib.error
import urllib.request

import ledger
import parse_scored

# Both credentials arrive ONLY through the environment, from an environment
# profile. Neither is ever an argument, in the definition, or in the repo.
BROKER_KEY_ENV = "SOURCING_BROKER_KEY"
BROKER_URL_ENV = "SOURCING_BROKER_URL"


def publish_one(entry: dict, url: str, key: str) -> bool:
    """POST one row, keyed by its entry id. Returns whether it landed.

    A per-row failure is reported and skipped rather than raised: losing one row
    to a transient error should not discard the twenty that already published,
    and the id simply stays out of the ledger so the next run retries it.
    """
    body = json.dumps({"key": entry["id"], "fields": entry}).encode("utf-8")
    request = urllib.request.Request(
        url,
        data=body,
        headers={"Authorization": f"Bearer {key}", "Content-Type": "application/json"},
        method="POST",
    )
    try:
        with urllib.request.urlopen(request, timeout=30):
            return True
    except (urllib.error.HTTPError, urllib.error.URLError) as error:
        # The id is safe to print; the key never is.
        print(f"publish: {entry['id']} failed: {error}", flush=True)
        return False


def commit_ledger(path: str) -> None:
    """Commit the ledger, treating "nothing changed" as success.

    A run that published nothing new has an unchanged ledger, and failing there
    would turn a correct quiet run into a red one.
    """
    subprocess.run(["git", "add", "--", path], check=True)
    status = subprocess.run(
        ["git", "diff", "--cached", "--quiet", "--", path],
        check=False,
    )
    if status.returncode == 0:
        print("publish: ledger unchanged", flush=True)
        return
    subprocess.run(
        ["git", "commit", "-m", "chore(sourcing): record published entries"],
        check=True,
    )
    subprocess.run(["git", "push"], check=True)


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--scored", required=True)
    parser.add_argument("--min-score", required=True, type=float)
    parser.add_argument("--ledger", required=True)
    args = parser.parse_args()

    key, url = os.environ.get(BROKER_KEY_ENV), os.environ.get(BROKER_URL_ENV)
    if not key or not url:
        print(
            f"publish: {BROKER_KEY_ENV} and {BROKER_URL_ENV} must both be set; "
            "add them to this session's environment profile",
            file=sys.stderr,
        )
        return 1

    with open(args.scored, encoding="utf-8") as handle:
        # Defensive by design — see parse_scored. A step-2 output this cannot
        # read fails HERE, loudly, rather than publishing nothing quietly.
        entries = parse_scored.parse(handle.read())

    accepted = [entry for entry in entries if entry["score"] >= args.min_score]
    print(f"publish: {len(accepted)} of {len(entries)} met the threshold", flush=True)

    landed = [entry["id"] for entry in accepted if publish_one(entry, url, key)]
    if len(landed) != len(accepted):
        print(f"publish: {len(accepted) - len(landed)} row(s) will be retried next run", flush=True)

    ledger.save(args.ledger, ledger.extend(ledger.load(args.ledger), landed))
    commit_ledger(args.ledger)
    # A partial publish is NOT a failed run: the ledger records exactly what
    # landed, and the remainder is retried. Only an unreadable score payload or a
    # missing credential fails the step.
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
