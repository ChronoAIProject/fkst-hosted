#!/usr/bin/env bash
set -euo pipefail

BIN="${BIN:-/Users/auric/fkst-substrate/target/debug/fkst-framework}"
PKG_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

HOST="$TMP/host"
FAKEBIN="$TMP/bin"
RUNTIME="$TMP/runtime"
mkdir -p "$HOST" "$FAKEBIN" "$RUNTIME"

git -C "$HOST" init -q
git -C "$HOST" config user.email fkst-test@example.invalid
git -C "$HOST" config user.name "fkst test"
git -C "$HOST" commit --allow-empty -m "seed" >/dev/null

cat > "$FAKEBIN/gh" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
LOG="${FAKE_GH_LOG:?}"
printf 'ARGS:' >> "$LOG"
printf ' [%s]' "$@" >> "$LOG"
printf '\n' >> "$LOG"

if [[ "${1:-}" == "issue" && "${2:-}" == "list" ]]; then
  printf '[{"number":42,"title":"Bridge issue","url":"https://github.example/owner/x/issues/42","updatedAt":"2026-06-03T01:02:03Z","state":"OPEN"}]\n'
  exit 0
fi

if [[ "${1:-}" == "issue" && "${2:-}" == "view" ]]; then
  printf '{"comments":[{"body":"existing comment"}]}\n'
  exit 0
fi

if [[ "${1:-}" == "issue" && "${2:-}" == "comment" ]]; then
  body_file=""
  while [[ $# -gt 0 ]]; do
    if [[ "$1" == "--body-file" ]]; then
      body_file="$2"
      break
    fi
    shift
  done
  {
    printf 'BODY_BEGIN\n'
    cat "$body_file"
    printf 'BODY_END\n'
  } >> "$LOG"
  exit 0
fi

printf 'unexpected gh invocation\n' >&2
exit 9
EOF
chmod +x "$FAKEBIN/gh"

export PATH="$FAKEBIN:$PATH"
export FAKE_GH_LOG="$TMP/gh.log"
export FKST_GITHUB_REPO="owner/x"
export FKST_RUNTIME_ROOT="$RUNTIME"

poll_event='{"queue":"github_poll_tick","payload":{}}'
poll_out="$TMP/poll.out"
(
  cd "$HOST"
  "$BIN" run "$PKG_ROOT/departments/github_poll/main.lua" \
    --project-root "$HOST" \
    --package-root "$PKG_ROOT" \
    --event "$poll_event"
) >"$poll_out" 2>&1

grep -q 'RAISED:' "$poll_out"
python3 - "$poll_out" <<'PY'
import base64
import json
import re
import sys

text = open(sys.argv[1], encoding="utf-8").read()
match = re.search(r"RAISED:\s*([A-Za-z0-9_-]+)", text)
assert match, text
data = json.loads(base64.urlsafe_b64decode(match.group(1) + "=="))
assert data[0]["queue"] == "github_issue_seen", data
payload = data[0]["payload"]
assert payload["repo"] == "owner/x", payload
assert payload["issue_number"] == 42, payload
assert payload["dedup_key"] == "owner/x#42@2026-06-03T01:02:03Z", payload
PY

git -C "$HOST" log --oneline --grep='github-proxy:seen:issue:owner/x#42@2026-06-03T01:02:03Z' | grep -q .

poll_again="$TMP/poll-again.out"
(
  cd "$HOST"
  "$BIN" run "$PKG_ROOT/departments/github_poll/main.lua" \
    --project-root "$HOST" \
    --package-root "$PKG_ROOT" \
    --event "$poll_event"
) >"$poll_again" 2>&1
if grep -q 'RAISED:' "$poll_again"; then
  echo "duplicate inbound raise" >&2
  cat "$poll_again" >&2
  exit 1
fi

comment_event='{"queue":"github_issue_comment_request","payload":{"issue_number":42,"body":"fkst reply","dedup_key":"reply-42"}}'
: > "$FAKE_GH_LOG"
unset FKST_GITHUB_WRITE || true
dry_out="$TMP/dry.out"
(
  cd "$HOST"
  "$BIN" run "$PKG_ROOT/departments/github_comment/main.lua" \
    --project-root "$HOST" \
    --package-root "$PKG_ROOT" \
    --event "$comment_event"
) >"$dry_out" 2>&1
grep -q 'dry-run' "$dry_out"
if grep -Fq 'ARGS: [issue] [comment]' "$FAKE_GH_LOG"; then
  echo "dry-run mutated gh" >&2
  cat "$FAKE_GH_LOG" >&2
  exit 1
fi

: > "$FAKE_GH_LOG"
export FKST_GITHUB_WRITE=1
write_out="$TMP/write.out"
(
  cd "$HOST"
  "$BIN" run "$PKG_ROOT/departments/github_comment/main.lua" \
    --project-root "$HOST" \
    --package-root "$PKG_ROOT" \
    --event "$comment_event"
) >"$write_out" 2>&1

grep -Fq 'ARGS: [issue] [comment]' "$FAKE_GH_LOG"
grep -q '<!-- fkst:github-proxy:comment:reply-42 -->' "$FAKE_GH_LOG"
if grep -q 'github.com' "$FAKE_GH_LOG"; then
  echo "fake gh log unexpectedly references github.com" >&2
  cat "$FAKE_GH_LOG" >&2
  exit 1
fi

echo "integration ok"
