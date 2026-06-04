#!/usr/bin/env bash
# Read-only inbound dogfood: run github-proxy's github_poll against a REAL
# GitHub repo using the host's real `gh`, twice, to show:
#   1. emitted github_entity_changed events (decoded from the RAISED line),
#   2. the readable <RT>/cache/github-proxy tree,
#   3. second-run dedup (unchanged entities are not re-raised).
# Never writes to GitHub: FKST_GITHUB_WRITE is unset, only `gh ... list` runs.
#
# Usage:
#   scripts/dogfood.sh <owner/repo>          # e.g. ChronoAIProject/fkst-substrate
#   FKST_GITHUB_REPO=owner/repo scripts/dogfood.sh
set -euo pipefail

DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$DIR/.." && pwd)"
# shellcheck source=lib.sh
. "$DIR/lib.sh"
resolve_bin

REPO="${1:-${FKST_GITHUB_REPO:-}}"
if [ -z "$REPO" ]; then
  echo "usage: scripts/dogfood.sh <owner/repo>   (or set FKST_GITHUB_REPO)" >&2
  exit 1
fi
command -v gh >/dev/null 2>&1 || { echo "error: gh CLI not found / not authenticated" >&2; exit 1; }

PKG="$ROOT/packages/github-proxy"
DEPT="$PKG/departments/github_poll/main.lua"
EVENT='{"queue":"github_poll_tick","payload":{}}'
RT="$(mktemp -d "${TMPDIR:-/tmp}/fkst-ghproxy-dogfood.XXXXXX")"
export FKST_RUNTIME_ROOT="$RT"
export FKST_GITHUB_REPO="$REPO"
unset FKST_GITHUB_WRITE  # read-only: inbound poll only

echo "BIN=$BIN"
echo "REPO=$REPO"
echo "FKST_RUNTIME_ROOT=$RT  (read-only; FKST_GITHUB_WRITE unset)"

poll() {  # one-shot run of the github_poll department; prints its stdout
  "$BIN" run "$DEPT" --project-root "$ROOT" --package-root "$PKG" --event "$EVENT" 2>&1
}

decode_events() {  # stdin: a run's stdout -> pretty github_entity_changed events
  local b64
  b64="$(grep '^RAISED:' | sed 's/^RAISED: //' | tail -1 || true)"
  if [ -z "$b64" ]; then
    echo "  (no events raised)"
    return
  fi
  printf '%s' "$b64" | base64 -d 2>/dev/null | python3 -m json.tool
}

echo
echo "===== first run ====="
first="$(poll)"
echo "$first" | grep -vE '^RAISED:' || true   # logs, minus the base64 blob
echo "--- raised github_entity_changed (decoded) ---"
printf '%s\n' "$first" | decode_events

echo
echo "===== <RT>/cache (readable entity tree) ====="
find "$RT/cache" -type f 2>/dev/null | sort | while read -r f; do
  echo "  ${f#"$RT"/} = $(cat "$f")"
done

echo
echo "===== second run (same <RT>; unchanged entities should dedup) ====="
second="$(poll)"
raised_count="$(printf '%s\n' "$second" | grep -c '^RAISED:' || true)"
echo "  RAISED lines: $raised_count  (0 = all deduped, nothing re-raised)"

echo
echo "done. scratch left at: $RT  (safe to rm -rf)"
