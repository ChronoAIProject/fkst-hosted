#!/usr/bin/env python3
"""The cross-run ledger for the milestone-24 acceptance workflow.

The ledger is the ONLY state that survives between runs, and #5846 requires it
to be a committed file in the repository rather than a control-plane store --
that is what keeps the scheduled-workflow capability stateless.

It lives on its own branch, `cron-acceptance-ledger`, NOT on `develop`. Each run
works in a fresh clone of the default branch, so a ledger committed to `develop`
would need a pull request per run; `develop` is protected and the repository's
own rules forbid pushing to it directly. A dedicated branch is writable, carries
exactly two files, and keeps run bookkeeping out of the source history.

Writes go through git plumbing (hash-object / mktree / commit-tree) rather than
add+commit, so the run never switches the working tree it is executing from.

The commit is IDEMPOTENT: if the resulting tree matches the branch tip's tree,
nothing is committed. Re-running the same slot therefore cannot double-publish,
which is the property #5846 asks for by name.
"""

from __future__ import annotations

import json
import logging
import subprocess

LOG = logging.getLogger("ledger")

BRANCH = "cron-acceptance-ledger"
LEDGER_FILE = "ledger.json"
PUBLISHED_FILE = "published.json"

# Bound the ledger so a long-lived schedule cannot grow one file without limit.
# The window only ever consults the tail, so older ids are safe to drop.
MAX_IDS = 200


class LedgerError(RuntimeError):
    """A ledger operation failed in a way the run must not paper over."""


def _git(*args: str, check: bool = True, stdin: str | None = None) -> str:
    """Run one git command, returning stdout.

    Never `shell=True`: arguments reach git as argv, so an operator-supplied
    value cannot become shell syntax.
    """
    result = subprocess.run(
        ["git", *args],
        input=stdin,
        capture_output=True,
        text=True,
        check=False,
    )
    if check and result.returncode != 0:
        raise LedgerError(
            f"git {' '.join(args)} failed ({result.returncode}): {result.stderr.strip()}"
        )
    return result.stdout


def _tip() -> str | None:
    """The ledger branch's current commit, or None when it does not exist yet."""
    _git("fetch", "--quiet", "origin", BRANCH, check=False)
    revision = _git("rev-parse", "--verify", "--quiet", "FETCH_HEAD", check=False).strip()
    return revision or None


def read() -> tuple[list[str], dict[str, dict]]:
    """The published ids and the destination table, as of the branch tip.

    A missing branch means the first run and yields empty state. A branch that
    exists but whose contents will not parse is a HARD failure: treating
    corruption as "nothing published yet" would re-publish the entire history,
    the exact outcome the ledger exists to prevent.
    """
    tip = _tip()
    if tip is None:
        LOG.info("no %s branch yet; treating this as the first run", BRANCH)
        return [], {}

    ids = _read_json(tip, LEDGER_FILE, default=[])
    if not isinstance(ids, list) or not all(isinstance(entry, str) for entry in ids):
        raise LedgerError(f"{LEDGER_FILE} on {BRANCH} must be a JSON array of strings")

    published = _read_json(tip, PUBLISHED_FILE, default={})
    if not isinstance(published, dict):
        raise LedgerError(f"{PUBLISHED_FILE} on {BRANCH} must be a JSON object keyed by id")

    LOG.info("ledger at %s carries %d published id(s)", tip[:8], len(ids))
    return ids, published


def _read_json(tip: str, name: str, default):
    """One file's decoded content from a commit, or `default` when absent."""
    raw = _git("show", f"{tip}:{name}", check=False)
    if not raw.strip():
        return default
    try:
        return json.loads(raw)
    except json.JSONDecodeError as error:
        raise LedgerError(f"{name} on {BRANCH} is not valid JSON: {error}") from error


def write(ids: list[str], published: dict[str, dict], message: str) -> bool:
    """Commit the ledger and destination table. Returns False when unchanged."""
    trimmed = ids[-MAX_IDS:]
    entries = {
        LEDGER_FILE: json.dumps(trimmed, indent=2) + "\n",
        PUBLISHED_FILE: json.dumps(published, indent=2, sort_keys=True) + "\n",
    }

    lines = []
    for name, content in sorted(entries.items()):
        blob = _git("hash-object", "-w", "--stdin", stdin=content).strip()
        lines.append(f"100644 blob {blob}\t{name}")
    tree = _git("mktree", stdin="\n".join(lines) + "\n").strip()

    tip = _tip()
    if tip is not None and _git("rev-parse", f"{tip}^{{tree}}").strip() == tree:
        LOG.info("ledger tree is unchanged; nothing to commit (idempotent re-run)")
        return False

    parents = ["-p", tip] if tip else []
    commit = _git("commit-tree", tree, *parents, "-m", message).strip()
    _git("push", "--quiet", "origin", f"{commit}:refs/heads/{BRANCH}")
    LOG.info("committed ledger %s to %s (%d id(s) retained)", commit[:8], BRANCH, len(trimmed))
    return True
