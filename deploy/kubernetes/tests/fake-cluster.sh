#!/bin/sh
# shellcheck shell=sh

# A fake single-node cluster for the deployment gates, sourced rather than run.
#
# Two suites need the same fake `kubectl`: the verifier-decision suite
# (`audit-relay-verify-test.sh`) and the runbook smoke
# (`audit-runbook-smoke.sh`). They ask DIFFERENT questions of it — one enumerates
# the contracts the verifier must refuse, the other walks an operator's
# procedures end to end — but they need identical cluster behaviour, and two
# copies of a two-hundred-line stub would diverge on the first change.
#
# Sourcing this file provides:
#
#   $verifier         the path to verify-audit-relay.sh
#   reset_case        restore every FAKE_* knob to a healthy deployment
#   expect_success    run the verifier and require it to pass
#   expect_failure    run the verifier and require it to fail
#
# The stub answers only the invocations the verifier actually makes and exits
# non-zero on anything else, so a new verifier command shows up as a loud
# "unhandled fake kubectl invocation" rather than as a silently skipped check.

fake_cluster_dir=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
source_dir=$(CDPATH='' cd -- "$fake_cluster_dir/.." && pwd)
test_root=$(mktemp -d "${TMPDIR:-/tmp}/fkst-fake-cluster.XXXXXX")
trap 'rm -rf -- "$test_root"' EXIT HUP INT TERM

fake_bin="$test_root/bin"
log="$test_root/commands.log"
output="$test_root/output.log"
mkdir -p "$fake_bin"

cat >"$fake_bin/kubectl" <<'KUBECTL'
#!/bin/sh
set -eu
printf 'KUBECTL %s\n' "$*" >>"$FAKE_KUBECTL_LOG"
[ "$#" -ge 3 ] && [ "$1" = "--context" ] || {
  echo "fake kubectl rejected an invocation without explicit context" >&2
  exit 91
}
[ "$2" = "$FAKE_EXPECTED_CONTEXT" ] || {
  echo "fake kubectl rejected the wrong context" >&2
  exit 92
}
shift 2
command_line=" $* "

# A realistic exposition: every family carries its `# TYPE` declaration and at
# least one sample, because the verifier now demands both. `headers-only` mode
# reproduces the renderer that emits a family header before a loop that never
# runs — the exact shape a substring grep used to accept.
relay_metrics() {
  started=${FAKE_RECORDS_STARTED:-1}
  complete=${FAKE_RECORDS_COMPLETE:-2}
  dead=${FAKE_DEAD_LETTERS:-0}
  if [ -e "$FAKE_KUBECTL_STATE.after" ]; then
    started=${FAKE_RECORDS_STARTED_AFTER:-$started}
    complete=${FAKE_RECORDS_COMPLETE_AFTER:-$complete}
    dead=${FAKE_DEAD_LETTERS_AFTER:-$dead}
  fi
  cat <<METRICS
# TYPE fkst_audit_relay_up gauge
fkst_audit_relay_up 1
# TYPE fkst_audit_relay_ingress_ready gauge
fkst_audit_relay_ingress_ready ${FAKE_INGRESS_READY:-1}
# TYPE fkst_audit_relay_db_bytes gauge
fkst_audit_relay_db_bytes 1048576
# TYPE fkst_audit_relay_max_records gauge
fkst_audit_relay_max_records ${FAKE_MAX_RECORDS:-5000000}
# TYPE fkst_audit_relay_records gauge
fkst_audit_relay_records{state="started"} ${started}
fkst_audit_relay_records{state="complete"} ${complete}
# TYPE fkst_audit_relay_oldest_record_age_seconds gauge
fkst_audit_relay_oldest_record_age_seconds{state="complete"} 3
# TYPE fkst_audit_relay_capture_total counter
fkst_audit_relay_capture_total{result="accepted"} 4
fkst_audit_relay_capture_total{result="permanent"} 0
# TYPE fkst_audit_relay_dead_letters_total counter
fkst_audit_relay_dead_letters_total{reason="permanent"} ${dead}
# TYPE fkst_audit_relay_incomplete_total counter
fkst_audit_relay_incomplete_total 0
${FAKE_EXTRA_METRIC:-}
METRICS
}

plane_metrics() {
  mode=$1
  cat <<METRICS
# TYPE fkst_audit_required_rejections_total counter
fkst_audit_required_rejections_total{reason="audit_ingress_unavailable"} 0
# TYPE fkst_operations_activity_queries_total counter
fkst_operations_activity_queries_total{scope="personal",result="success"} 0
METRICS
  [ "$mode" = "missing-partial" ] || cat <<METRICS
# TYPE fkst_operations_activity_source_partial_total counter
fkst_operations_activity_source_partial_total{source="relay"} 0
METRICS
  echo '# TYPE fkst_operations_sandbox_inventory_requests_total counter'
  # The header-only shape: declared, never sampled.
  [ "$mode" = "headers-only" ] ||
    echo 'fkst_operations_sandbox_inventory_requests_total{backend="kubernetes",scope="personal",result="ok"} 0'
  cat <<METRICS
# TYPE fkst_session_access_registry_generation_state gauge
fkst_session_access_registry_generation_state{state="ready"} 1
METRICS
}

case "$command_line" in
  *" config view "*) exit 0 ;;
  *" auth can-i "*) printf '%s' "${FAKE_CAN_I:-no}"; exit 0 ;;
  *" --output name "*) exit 0 ;;
  *" wait "*) exit 0 ;;
  *" rollout status "*) exit 0 ;;
  *" rollout restart "*) : >"$FAKE_KUBECTL_STATE.after"; exit 0 ;;
  *" scale deployment/fkst-audit-relay --replicas=0 "*)
    : >"$FAKE_KUBECTL_STATE.scaled"; exit 0 ;;
  *" scale deployment/fkst-audit-relay --replicas=1 "*)
    rm -f -- "$FAKE_KUBECTL_STATE.scaled"; : >"$FAKE_KUBECTL_STATE.after"; exit 0 ;;
  *"{{range \$key, \$_ := .data}}"*)
    printf '%b' "${FAKE_SECRET_KEYS:-FKST_AUDIT_RELAY_WRITE_TOKEN\nFKST_AUDIT_RELAY_READ_TOKEN\nFKST_POSTHOG_PROJECT_TOKEN\nFKST_POSTHOG_QUERY_API_KEY\n}"
    exit 0
    ;;
  *".automountServiceAccountToken"*) printf '%s' "${FAKE_AUTOMOUNT:-false}"; exit 0 ;;
  *"data.FKST_AUDIT_RELAY_WRITE_TOKEN"*) printf '%s' "${FAKE_CONFIGMAP_TOKEN:-}"; exit 0 ;;
  *"data.FKST_POSTHOG_PROJECT_TOKEN"*) printf '%s' "${FAKE_CONFIGMAP_TOKEN:-}"; exit 0 ;;
  *"data.FKST_"*_TOKEN"}"*) printf ''; exit 0 ;;
  *"data.FKST_POSTHOG_QUERY_API_KEY"*) printf ''; exit 0 ;;
  *".status.phase"*) printf '%s' "${FAKE_PVC_PHASE:-Bound}"; exit 0 ;;
  *".spec.storageClassName"*) printf '%s' "${FAKE_STORAGE_CLASS:-standard}"; exit 0 ;;
  # `${FAKE_CAPACITY-…}` without the colon on purpose: an explicitly EMPTY value
  # is a claim that reports no capacity, which is a state the verifier must
  # refuse and `:-` would silently turn back into a healthy one.
  *".status.capacity.storage"*) printf '%s' "${FAKE_CAPACITY-20Gi}"; exit 0 ;;
  *".spec.replicas"*) printf '%s' "${FAKE_REPLICAS:-1}"; exit 0 ;;
  *".spec.strategy.type"*) printf '%s' "${FAKE_STRATEGY:-Recreate}"; exit 0 ;;
  *"configmap fkst-audit-relay-config --output jsonpath={.data.FKST_AUDIT_INCOMPLETE_GRACE_SECS}"*)
    printf '%s' "${FAKE_RELAY_GRACE:-420}"; exit 0 ;;
  *"configmap fkst-control-plane-config --output jsonpath={.data.FKST_AUDIT_INCOMPLETE_GRACE_SECS}"*)
    printf '%s' "${FAKE_PLANE_GRACE:-420}"; exit 0 ;;
  *".data.FKST_AUDIT_DELIVERY_MODE"*) printf '%s' "${FAKE_MODE:-required}"; exit 0 ;;
  *".data.FKST_AUDIT_RELAY_URL"*) printf '%s' "${FAKE_RELAY_URL:-http://fkst-audit-relay.chronoai-fkst.svc.cluster.local}"; exit 0 ;;
  *" exec deployment/fkst-frontend "*)
    # The probe reports a sentinel; the interesting cases are the ones where it
    # cannot report at all, which used to read as "the cage holds".
    case "${FAKE_FRONTEND_PROBE:-blocked}" in
      blocked) echo FKST_PROBE_BLOCKED; exit 0 ;;
      reached) echo FKST_PROBE_REACHED; exit 0 ;;
      no-tool) echo FKST_PROBE_NO_TOOL; exit 0 ;;
      no-dns) echo FKST_PROBE_NO_DNS; exit 0 ;;
      silent) exit 0 ;;
      exec-error) echo "error: unable to upgrade connection" >&2; exit 1 ;;
    esac
    exit 0
    ;;
  *" exec deployment/fkst-control-plane "*)
    case "$command_line" in
      *"/ready"*) printf '%s' "${FAKE_RELAY_READY:-{\"ready\":true\}}"; exit 0 ;;
      *"127.0.0.1:8080/metrics"*)
        if [ -e "$FAKE_KUBECTL_STATE.scaled" ]; then
          plane_metrics "${FAKE_PLANE_MODE_DEGRADED:-full}"
        else
          plane_metrics "${FAKE_PLANE_MODE:-full}"
        fi
        exit 0
        ;;
      *"/metrics"*)
        # The relay is unreachable while it is scaled to zero, exactly as a real
        # one would be — the verifier must never read it in that window.
        if [ -e "$FAKE_KUBECTL_STATE.scaled" ]; then
          echo "connection refused" >&2
          exit 1
        fi
        relay_metrics
        exit 0
        ;;
    esac
    ;;
esac
echo "unhandled fake kubectl invocation: $*" >&2
exit 95
KUBECTL
chmod +x "$fake_bin/kubectl"

export PATH="$fake_bin:$PATH"
export FAKE_KUBECTL_LOG="$log"
export FAKE_KUBECTL_STATE="$test_root/state"
export FAKE_EXPECTED_CONTEXT="kind-opensandbox-local"
verifier="$source_dir/verify-audit-relay.sh"

reset_case() {
  : >"$log"
  rm -f -- "$FAKE_KUBECTL_STATE.after" "$FAKE_KUBECTL_STATE.scaled"
  FAKE_CAN_I=no
  FAKE_AUTOMOUNT=false
  FAKE_CONFIGMAP_TOKEN=""
  FAKE_PVC_PHASE=Bound
  FAKE_STORAGE_CLASS=standard
  FAKE_CAPACITY=20Gi
  FAKE_REPLICAS=1
  FAKE_STRATEGY=Recreate
  FAKE_RELAY_GRACE=420
  FAKE_PLANE_GRACE=420
  FAKE_MODE=required
  FAKE_RELAY_URL=http://fkst-audit-relay.chronoai-fkst.svc.cluster.local
  FAKE_FRONTEND_PROBE=blocked
  FAKE_RECORDS_STARTED=1
  FAKE_RECORDS_COMPLETE=2
  FAKE_RECORDS_STARTED_AFTER=1
  FAKE_RECORDS_COMPLETE_AFTER=2
  FAKE_DEAD_LETTERS=0
  FAKE_DEAD_LETTERS_AFTER=0
  FAKE_INGRESS_READY=1
  FAKE_MAX_RECORDS=5000000
  FAKE_EXTRA_METRIC=""
  FAKE_SECRET_KEYS='FKST_AUDIT_RELAY_WRITE_TOKEN\nFKST_AUDIT_RELAY_READ_TOKEN\nFKST_POSTHOG_PROJECT_TOKEN\nFKST_POSTHOG_QUERY_API_KEY\n'
  FAKE_PLANE_MODE=full
  FAKE_PLANE_MODE_DEGRADED=full
  export FAKE_CAN_I FAKE_AUTOMOUNT FAKE_CONFIGMAP_TOKEN FAKE_PVC_PHASE FAKE_STORAGE_CLASS
  export FAKE_CAPACITY FAKE_REPLICAS FAKE_STRATEGY FAKE_RELAY_GRACE FAKE_PLANE_GRACE
  export FAKE_MODE FAKE_RELAY_URL FAKE_FRONTEND_PROBE FAKE_SECRET_KEYS
  export FAKE_RECORDS_STARTED FAKE_RECORDS_COMPLETE FAKE_EXTRA_METRIC
  export FAKE_RECORDS_STARTED_AFTER FAKE_RECORDS_COMPLETE_AFTER
  export FAKE_DEAD_LETTERS FAKE_DEAD_LETTERS_AFTER FAKE_INGRESS_READY FAKE_MAX_RECORDS
  export FAKE_PLANE_MODE FAKE_PLANE_MODE_DEGRADED
}

expect_success() {
  name=$1
  shift
  if ! "$verifier" --context kind-opensandbox-local "$@" >"$output" 2>&1; then
    echo "expected success: $name" >&2
    sed 's/^/  /' "$output" >&2
    exit 1
  fi
  echo "ok  $name"
}

expect_failure() {
  name=$1
  shift
  if "$verifier" --context kind-opensandbox-local "$@" >"$output" 2>&1; then
    echo "expected failure: $name" >&2
    sed 's/^/  /' "$output" >&2
    exit 1
  fi
  echo "ok  $name"
}
