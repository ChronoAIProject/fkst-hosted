#!/bin/sh
# git credential helper for fkst-hosted substrate sessions (Model B, issue #359).
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
# The control plane rotates the token file in place (§5.4). The helper re-reads it
# on every op, so a rotation is picked up with no process restart — this helper
# holds no key, cannot mint, and needs no just-in-time mint path of its own.

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

token=$(extract_field token "$token_file")
if [ -z "$token" ]; then
    echo "git-credential-fkst: no token in $token_file" >&2
    exit 0
fi

printf 'username=x-access-token\n'
printf 'password=%s\n' "$token"
