#!/bin/sh
# run-e2e.sh — the fkst-hosted v1 happy-path smoke test (issue #21).
#
# Black-box client of the deployed HTTP API: health-gate, create the
# e2e-minimal package, start a session, poll to `running`, stop it, poll to
# `stopped`. Exits 0 only when `stopped` is reached. Dependencies: curl + jq.
#
# POSIX/pipefail resolution: `pipefail` is not POSIX, so this script is
# written with ZERO pipelines — every curl writes its body to a temp file
# (`-o file -w '%{http_code}'`) and jq reads files directly. `set -o
# pipefail` is still enabled opportunistically below for shells that have it
# (defense-in-depth should a pipeline ever sneak in), but correctness never
# depends on it.
#
# Output contract: human-readable progress banners (`[e2e] step N: ...`) and
# diagnostics go to STDERR; the only STDOUT is the final session JSON.
#
# Env vars (all optional; timeouts/interval are positive-integer seconds):
#   FKST_HOSTED_BASE_URL  base URL, no trailing slash   (default http://localhost:8080)
#   E2E_HEALTH_TIMEOUT    max wait for GET /health      (default 60)
#   E2E_START_TIMEOUT     max wait to reach `running`   (default 120)
#   E2E_STOP_TIMEOUT      max wait to reach `stopped`   (default 60)
#   E2E_POLL_INTERVAL     sleep between status polls    (default 2)
#
# Exit codes (one per phase):
#   0  the session reached `stopped`
#   1  usage error: missing curl/jq, bad env value, missing fixture
#   2  health gate failed (service unreachable / not ok within timeout)
#   3  package create failed (anything other than 201/409)
#   4  session start failed (409 live-lease conflict, or any non-201)
#   5  start phase failed (`failed` status, illegal status, or timeout)
#   6  stop request failed (anything other than 202)
#   7  stop phase failed (`failed` status, illegal status, or timeout)
set -eu
# Opportunistic pipefail (see header); ignored on shells without it.
# shellcheck disable=SC3040 # deliberate non-POSIX probe, guarded by the subshell
(set -o pipefail) 2>/dev/null && set -o pipefail || true

# ---------------------------------------------------------------- config --
FKST_HOSTED_BASE_URL="${FKST_HOSTED_BASE_URL:-http://localhost:8080}"
E2E_HEALTH_TIMEOUT="${E2E_HEALTH_TIMEOUT:-60}"
E2E_START_TIMEOUT="${E2E_START_TIMEOUT:-120}"
E2E_STOP_TIMEOUT="${E2E_STOP_TIMEOUT:-60}"
E2E_POLL_INTERVAL="${E2E_POLL_INTERVAL:-2}"

API_BASE="${FKST_HOSTED_BASE_URL}/api/v1"
PACKAGE_NAME="e2e-minimal"
FIXTURE_REL="departments/hello/main.lua"

log() { printf '[e2e] %s\n' "$*" >&2; }
die() {
    code="$1"
    shift
    printf '[e2e] ERROR: %s\n' "$*" >&2
    exit "$code"
}

# require_int_env NAME — fail fast unless $NAME is a positive integer.
require_int_env() {
    name="$1"
    eval "value=\"\$${name}\""
    # shellcheck disable=SC2154 # assigned by the eval above
    case "$value" in
    '' | *[!0-9]*)
        die 1 "$name must be a positive integer (seconds), got: '$value'"
        ;;
    esac
    if [ "$value" -lt 1 ]; then
        die 1 "$name must be a positive integer (seconds), got: '$value'"
    fi
}

command -v curl >/dev/null 2>&1 || die 1 "curl is required but not on PATH"
command -v jq >/dev/null 2>&1 || die 1 "jq is required but not on PATH"
require_int_env E2E_HEALTH_TIMEOUT
require_int_env E2E_START_TIMEOUT
require_int_env E2E_STOP_TIMEOUT
require_int_env E2E_POLL_INTERVAL

# Resolve the fixture relative to this script so it runs from any cwd.
FIXTURE_DIR="$(CDPATH='' cd -- "$(dirname -- "$0")/../../backend/tests/fixtures/e2e-minimal" && pwd)"
FIXTURE="$FIXTURE_DIR/$FIXTURE_REL"
[ -f "$FIXTURE" ] || die 1 "fixture not found: $FIXTURE"

WORK_DIR="$(mktemp -d)"
trap 'rm -rf "$WORK_DIR"' EXIT INT TERM
BODY_FILE="$WORK_DIR/body.json"
REQ_FILE="$WORK_DIR/request.json"

SESSION_ID=''

# ------------------------------------------------------------------ http --
# http METHOD URL [json-request-file] — body lands in $BODY_FILE, status
# code in $HTTP_CODE ('000' on transport failure, e.g. connection refused).
http() {
    method="$1"
    url="$2"
    req="${3:-}"
    : >"$BODY_FILE"
    if [ -n "$req" ]; then
        HTTP_CODE="$(curl -sS -X "$method" -H 'Content-Type: application/json' \
            --data @"$req" -o "$BODY_FILE" -w '%{http_code}' "$url" 2>>"$WORK_DIR/curl.err")" ||
            HTTP_CODE='000'
    else
        HTTP_CODE="$(curl -sS -X "$method" -o "$BODY_FILE" -w '%{http_code}' \
            "$url" 2>>"$WORK_DIR/curl.err")" || HTTP_CODE='000'
    fi
}

# http_failure METHOD URL EXPECTED — diagnostics for an unexpected code.
http_failure() {
    printf '[e2e] %s %s: expected %s, got %s\n' "$1" "$2" "$3" "$HTTP_CODE" >&2
    printf '[e2e] response body:\n' >&2
    cat "$BODY_FILE" >&2
    printf '\n' >&2
    if [ -s "$WORK_DIR/curl.err" ]; then
        printf '[e2e] curl errors:\n' >&2
        cat "$WORK_DIR/curl.err" >&2
    fi
}

# dump_session — fetch and print the final session JSON to STDOUT (the only
# stdout this script produces), so status/pod_id/pid/runtime_dir/error are
# always captured machine-readably.
dump_session() {
    if [ -z "$SESSION_ID" ]; then
        log 'no session id obtained; nothing to dump'
        return 0
    fi
    http GET "$API_BASE/sessions/$SESSION_ID"
    if [ "$HTTP_CODE" = '200' ]; then
        cat "$BODY_FILE"
        printf '\n'
    else
        log "could not fetch the final session (HTTP $HTTP_CODE)"
        cat "$BODY_FILE" >&2
        printf '\n' >&2
    fi
}

now() { date +%s; }

# ----------------------------------------------------------------- steps --

wait_for_health() {
    log "step 1: waiting for $FKST_HOSTED_BASE_URL/health (up to ${E2E_HEALTH_TIMEOUT}s)"
    deadline=$(($(now) + E2E_HEALTH_TIMEOUT))
    while [ "$(now)" -le "$deadline" ]; do
        http GET "$FKST_HOSTED_BASE_URL/health"
        if [ "$HTTP_CODE" = '200' ]; then
            health_status="$(jq -r '.status // empty' "$BODY_FILE" 2>/dev/null)" || health_status=''
            if [ "$health_status" = 'ok' ]; then
                log 'step 1: health ok'
                return 0
            fi
        fi
        sleep "$E2E_POLL_INTERVAL"
    done
    http_failure GET "$FKST_HOSTED_BASE_URL/health" '200 {"status":"ok"}'
    die 2 "E2E_HEALTH_TIMEOUT (${E2E_HEALTH_TIMEOUT}s) exceeded waiting for a healthy service"
}

create_package() {
    log "step 2: creating package '$PACKAGE_NAME' from $FIXTURE"
    # Build the body with jq so the lua is JSON-escaped, never hand-built.
    jq -n \
        --arg name "$PACKAGE_NAME" \
        --arg path "$FIXTURE_REL" \
        --rawfile lua "$FIXTURE" \
        '{ name: $name, files: [ { path: $path, content: $lua } ], composed_deps: [] }' \
        >"$REQ_FILE"
    http POST "$API_BASE/packages" "$REQ_FILE"
    case "$HTTP_CODE" in
    201) log 'step 2: package created (201)' ;;
    409) log 'step 2: package already exists (409) — fine on re-run' ;;
    *)
        http_failure POST "$API_BASE/packages" '201 or 409'
        die 3 "package create failed with HTTP $HTTP_CODE"
        ;;
    esac
}

start_session() {
    log "step 3: starting a session for '$PACKAGE_NAME'"
    jq -n --arg name "$PACKAGE_NAME" '{ package_name: $name }' >"$REQ_FILE"
    http POST "$API_BASE/sessions" "$REQ_FILE"
    if [ "$HTTP_CODE" = '409' ]; then
        printf '[e2e] CONFLICT: a live session already holds the lease for %s.\n' "$PACKAGE_NAME" >&2
        printf '[e2e] Stop the stale session first (POST %s/sessions/{id}/stop), then re-run.\n' "$API_BASE" >&2
        printf '[e2e] response body:\n' >&2
        cat "$BODY_FILE" >&2
        printf '\n' >&2
        die 4 'session start conflicted with a live session (409)'
    fi
    if [ "$HTTP_CODE" != '201' ]; then
        http_failure POST "$API_BASE/sessions" '201'
        die 4 "session start failed with HTTP $HTTP_CODE"
    fi
    SESSION_ID="$(jq -r '.id // empty' "$BODY_FILE")"
    session_status="$(jq -r '.status // empty' "$BODY_FILE")"
    if [ -z "$SESSION_ID" ]; then
        http_failure POST "$API_BASE/sessions" '201 with a non-empty id'
        die 4 'session start answered 201 without a session id'
    fi
    if [ "$session_status" != 'pending' ]; then
        http_failure POST "$API_BASE/sessions" "201 with status 'pending'"
        die 4 "fresh session reported status '$session_status', expected 'pending'"
    fi
    log "step 3: session $SESSION_ID created (pending)"
}

poll_until_running() {
    log "step 4: polling session $SESSION_ID to 'running' (up to ${E2E_START_TIMEOUT}s)"
    deadline=$(($(now) + E2E_START_TIMEOUT))
    while [ "$(now)" -le "$deadline" ]; do
        http GET "$API_BASE/sessions/$SESSION_ID"
        if [ "$HTTP_CODE" != '200' ]; then
            http_failure GET "$API_BASE/sessions/$SESSION_ID" '200'
            dump_session
            die 5 "session poll failed with HTTP $HTTP_CODE"
        fi
        session_status="$(jq -r '.status // empty' "$BODY_FILE")"
        case "$session_status" in
        running)
            log 'step 4: session is running'
            return 0
            ;;
        pending | validating)
            log "step 4: status=$session_status ..."
            ;;
        failed)
            session_error="$(jq -r '.error // "null"' "$BODY_FILE")"
            log "step 4: session FAILED during start: $session_error"
            dump_session
            die 5 'session reached failed while waiting for running'
            ;;
        *)
            log "step 4: illegal status '$session_status' during the start phase"
            dump_session
            die 5 "unexpected session status '$session_status' while waiting for running"
            ;;
        esac
        sleep "$E2E_POLL_INTERVAL"
    done
    dump_session
    die 5 "E2E_START_TIMEOUT (${E2E_START_TIMEOUT}s) exceeded waiting for status 'running'"
}

stop_session() {
    log "step 5: stopping session $SESSION_ID"
    http POST "$API_BASE/sessions/$SESSION_ID/stop"
    if [ "$HTTP_CODE" != '202' ]; then
        http_failure POST "$API_BASE/sessions/$SESSION_ID/stop" '202'
        dump_session
        die 6 "session stop failed with HTTP $HTTP_CODE"
    fi
    log 'step 5: stop accepted (202)'
}

poll_until_stopped() {
    log "step 6: polling session $SESSION_ID to 'stopped' (up to ${E2E_STOP_TIMEOUT}s)"
    deadline=$(($(now) + E2E_STOP_TIMEOUT))
    while [ "$(now)" -le "$deadline" ]; do
        http GET "$API_BASE/sessions/$SESSION_ID"
        if [ "$HTTP_CODE" != '200' ]; then
            http_failure GET "$API_BASE/sessions/$SESSION_ID" '200'
            dump_session
            die 7 "session poll failed with HTTP $HTTP_CODE"
        fi
        session_status="$(jq -r '.status // empty' "$BODY_FILE")"
        case "$session_status" in
        stopped)
            log 'step 6: session is stopped'
            return 0
            ;;
        stopping)
            log 'step 6: status=stopping ...'
            ;;
        failed)
            session_error="$(jq -r '.error // "null"' "$BODY_FILE")"
            log "step 6: session FAILED during stop: $session_error"
            dump_session
            die 7 'session reached failed while waiting for stopped'
            ;;
        *)
            log "step 6: illegal status '$session_status' during the stop phase"
            dump_session
            die 7 "unexpected session status '$session_status' while waiting for stopped"
            ;;
        esac
        sleep "$E2E_POLL_INTERVAL"
    done
    dump_session
    die 7 "E2E_STOP_TIMEOUT (${E2E_STOP_TIMEOUT}s) exceeded waiting for status 'stopped'"
}

# ------------------------------------------------------------------ main --
wait_for_health
create_package
start_session
poll_until_running
stop_session
poll_until_stopped
log 'step 7: happy path complete — final session JSON on stdout'
dump_session
exit 0
