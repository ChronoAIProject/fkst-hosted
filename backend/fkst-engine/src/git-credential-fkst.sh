#!/bin/sh
# git credential helper for fkst-hosted goal sessions (issue #107).
#
# git invokes this as `<script> get|store|erase`, feeding `key=value` lines on
# stdin terminated by a blank line. Only `get` emits output; `store`/`erase`
# (and any unknown op) exit 0 silently. On `get` we print EXACTLY two lines to
# stdout — `username=x-access-token` and `password=<token>` — and send all
# diagnostics to stderr, because git parses stdout.
#
# The token lives in the JSON file named by $FKST_GITHUB_TOKEN_FILE:
#   {"token":"ghs_...","expires_at":"<RFC3339>"}
# We parse it with anchored grep/sed (no jq/python3 dependency): the token
# charset and the RFC3339 stamp contain no quotes, so a `"key":"<non-quote>*"`
# extraction is safe and non-greedy.
#
# This helper holds NO key and cannot mint. When the token is within the safety
# window of expiry it drops a nonce-bearing request file ($FKST_GITHUB_TOKEN_FILE
# + ".request") that the driver's poller services (mint -> atomic rewrite ->
# delete the request file), then waits — bounded — for that deletion before
# re-reading. If no refresh arrives in the window it falls back to the current
# token rather than failing git hard (the periodic backstop still covers true
# expiry).

set -u

op="${1:-}"

# Only `get` produces credentials; everything else is a silent success.
if [ "$op" != "get" ]; then
    exit 0
fi

# Drain stdin (the key=value request block) so git's pipe never blocks; we do
# not need its contents — the per-host git config already scopes us to github.
while IFS= read -r _line; do
    [ -z "$_line" ] && break
done

token_file="${FKST_GITHUB_TOKEN_FILE:-}"
if [ -z "$token_file" ] || [ ! -r "$token_file" ]; then
    echo "git-credential-fkst: token file unset or unreadable" >&2
    exit 0
fi

# Extract a JSON string field by key, anchored, non-greedy (no quotes in value).
extract_field() {
    field="$1"
    file="$2"
    grep -o "\"${field}\"[[:space:]]*:[[:space:]]*\"[^\"]*\"" "$file" 2>/dev/null \
        | head -n 1 \
        | sed "s/.*\"${field}\"[[:space:]]*:[[:space:]]*\"\\([^\"]*\\)\".*/\\1/"
}

# Seconds until the RFC3339 expiry (negative => already expired). Empty when the
# stamp is missing or `date` cannot parse it — the caller then skips the JIT path
# and just serves the current token.
seconds_until_expiry() {
    exp="$1"
    [ -z "$exp" ] && return 0
    now_epoch=$(date -u +%s 2>/dev/null) || return 0
    # GNU date: -d; BSD/macOS date: -j -f. Try both, quietly.
    exp_epoch=$(date -u -d "$exp" +%s 2>/dev/null \
        || date -u -j -f "%Y-%m-%dT%H:%M:%S" "${exp%%.*}" +%s 2>/dev/null \
        || date -u -j -f "%Y-%m-%dT%H:%M:%SZ" "$exp" +%s 2>/dev/null) || return 0
    [ -z "$exp_epoch" ] && return 0
    echo $((exp_epoch - now_epoch))
}

# Safety window (seconds): re-mint if the token expires within this. The driver
# can override via env for tests; default 5 minutes.
window="${FKST_GITHUB_MINT_WINDOW_SECS:-300}"
# Bounded wait (seconds) for the driver to service a JIT request.
wait_secs="${FKST_GITHUB_MINT_WAIT_SECS:-10}"

expires_at=$(extract_field expires_at "$token_file")
remaining=$(seconds_until_expiry "$expires_at")

# JIT re-mint: only when we have a parseable remaining lifetime that is within
# the window. A request needs the per-session nonce; without it we cannot prove
# the request to the driver, so we skip straight to serving the current token.
if [ -n "${remaining:-}" ] && [ "$remaining" -lt "$window" ] 2>/dev/null; then
    nonce="${FKST_GITHUB_MINT_NONCE:-}"
    if [ -n "$nonce" ]; then
        request_file="${token_file}.request"
        # Write atomically: tmp + mv, so the poller never reads a partial nonce.
        printf '%s\n' "$nonce" > "${request_file}.tmp" 2>/dev/null \
            && mv "${request_file}.tmp" "$request_file" 2>/dev/null
        # Wait (bounded) for the driver to consume the request (delete it) after
        # rewriting the token file. Keying on deletion — not mtime — avoids a
        # false positive from an unrelated periodic refresh.
        waited=0
        while [ "$waited" -lt "$wait_secs" ]; do
            [ ! -e "$request_file" ] && break
            sleep 1
            waited=$((waited + 1))
        done
        # Best-effort cleanup if the driver never serviced it.
        rm -f "$request_file" "${request_file}.tmp" 2>/dev/null
    fi
fi

token=$(extract_field token "$token_file")
if [ -z "$token" ]; then
    echo "git-credential-fkst: no token in $token_file" >&2
    exit 0
fi

printf 'username=x-access-token\n'
printf 'password=%s\n' "$token"
