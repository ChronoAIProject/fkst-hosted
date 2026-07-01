#!/bin/sh
# gh PATH shim for Model B substrate sessions (issue #359 §5.2).
#
# `gh` (used by the substrate's github-proxy package for issue ops + `gh pr
# create`) does NOT consult the git credential helper, so it cannot pick up the
# rotating App token the way `git` does. This shim — installed EARLY on PATH so a
# bare `gh` resolves here, never overwriting the real /usr/bin/gh — reads the
# CURRENT token from the mounted rotating token file on EVERY invocation and
# exports it as GH_TOKEN before exec'ing the real gh. One control-plane Secret
# rewrite (§5.4) therefore refreshes gh and git alike, with no in-pod refresh loop.
#
# The token file is the JSON `{"token":"ghs_...","expires_at":"<RFC3339>"}` named
# by $FKST_GITHUB_TOKEN_FILE (the same file the git credential helper reads). We
# extract the token with anchored grep/sed (no jq dependency); the token charset
# contains no quotes, so a non-greedy `"token":"<non-quote>*"` extraction is safe.
# GH_TOKEN is exported only into the child gh process and is never echoed.

set -u

token_file="${FKST_GITHUB_TOKEN_FILE:-}"
if [ -n "$token_file" ] && [ -r "$token_file" ]; then
    GH_TOKEN=$(grep -o '"token"[[:space:]]*:[[:space:]]*"[^"]*"' "$token_file" 2>/dev/null \
        | head -n 1 \
        | sed 's/.*"token"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/')
    if [ -n "$GH_TOKEN" ]; then
        export GH_TOKEN
    fi
fi

exec /usr/bin/gh "$@"
