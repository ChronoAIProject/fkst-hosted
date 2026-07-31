#!/bin/sh
set -eu

# Drives verify-audit-relay.sh against a fake cluster.
#
# The point is not to test kubectl — it is to prove the verifier's DECISIONS: it
# passes a correctly deployed relay, and it fails closed on each contract this
# milestone exists to enforce (a credential in a ConfigMap, a reachable relay
# identity, an unbound claim, a disagreeing grace, a NetworkPolicy that lets the
# frontend through, an identity token in the exposition, records lost across a
# restart). Every one of those is a mistake a manifest review can miss and a live
# cluster would otherwise reveal only during an incident.

script_dir=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
source_dir=$(CDPATH='' cd -- "$script_dir/.." && pwd)
test_root=$(mktemp -d "${TMPDIR:-/tmp}/fkst-relay-test.XXXXXX")
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

relay_metrics() {
  cat <<METRICS
fkst_audit_relay_up 1
fkst_audit_relay_ingress_ready 1
fkst_audit_relay_db_bytes 1048576
fkst_audit_relay_records{state="started"} ${FAKE_RECORDS_STARTED:-1}
fkst_audit_relay_records{state="complete"} ${FAKE_RECORDS_COMPLETE:-2}
fkst_audit_relay_oldest_record_age_seconds{state="complete"} 3
fkst_audit_relay_dead_letters_total{reason="permanent"} 0
fkst_audit_relay_incomplete_total 0
${FAKE_EXTRA_METRIC:-}
METRICS
}

case "$command_line" in
  *" config view "*) exit 0 ;;
  *" auth can-i "*) printf '%s' "${FAKE_CAN_I:-no}"; exit 0 ;;
  *" --output name "*) exit 0 ;;
  *" wait "*) exit 0 ;;
  *" rollout status "*) exit 0 ;;
  *" rollout restart "*) : >"$FAKE_KUBECTL_STATE.restarted"; exit 0 ;;
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
  *".status.capacity.storage"*) printf '%s' "${FAKE_CAPACITY:-20Gi}"; exit 0 ;;
  *".spec.replicas"*) printf '%s' "${FAKE_REPLICAS:-1}"; exit 0 ;;
  *".spec.strategy.type"*) printf '%s' "${FAKE_STRATEGY:-Recreate}"; exit 0 ;;
  *"configmap fkst-audit-relay-config --output jsonpath={.data.FKST_AUDIT_INCOMPLETE_GRACE_SECS}"*)
    printf '%s' "${FAKE_RELAY_GRACE:-420}"; exit 0 ;;
  *"configmap fkst-control-plane-config --output jsonpath={.data.FKST_AUDIT_INCOMPLETE_GRACE_SECS}"*)
    printf '%s' "${FAKE_PLANE_GRACE:-420}"; exit 0 ;;
  *".data.FKST_AUDIT_DELIVERY_MODE"*) printf '%s' "${FAKE_MODE:-required}"; exit 0 ;;
  *".data.FKST_AUDIT_RELAY_URL"*) printf '%s' "${FAKE_RELAY_URL:-http://fkst-audit-relay.chronoai-fkst.svc.cluster.local}"; exit 0 ;;
  *" exec deployment/fkst-frontend "*)
    [ "${FAKE_FRONTEND_REACHES_RELAY:-false}" = "true" ] || exit 7
    printf '{"ready":true}'
    exit 0
    ;;
  *" exec deployment/fkst-control-plane "*)
    case "$command_line" in
      *"/ready"*) printf '%s' "${FAKE_RELAY_READY:-{\"ready\":true\}}"; exit 0 ;;
      *"127.0.0.1:8080/metrics"*)
        printf '%s\n' "${FAKE_PLANE_METRICS:-fkst_audit_required_rejections_total fkst_operations_activity_queries_total fkst_operations_sandbox_inventory_requests_total fkst_session_access_registry_generation_state}"
        exit 0
        ;;
      *"/metrics"*)
        if [ -e "$FAKE_KUBECTL_STATE.restarted" ]; then
          FAKE_RECORDS_STARTED=${FAKE_RECORDS_STARTED_AFTER:-$FAKE_RECORDS_STARTED}
          FAKE_RECORDS_COMPLETE=${FAKE_RECORDS_COMPLETE_AFTER:-$FAKE_RECORDS_COMPLETE}
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
  rm -f -- "$FAKE_KUBECTL_STATE.restarted"
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
  FAKE_FRONTEND_REACHES_RELAY=false
  FAKE_RECORDS_STARTED=1
  FAKE_RECORDS_COMPLETE=2
  FAKE_RECORDS_STARTED_AFTER=1
  FAKE_RECORDS_COMPLETE_AFTER=2
  FAKE_EXTRA_METRIC=""
  FAKE_SECRET_KEYS='FKST_AUDIT_RELAY_WRITE_TOKEN\nFKST_AUDIT_RELAY_READ_TOKEN\nFKST_POSTHOG_PROJECT_TOKEN\nFKST_POSTHOG_QUERY_API_KEY\n'
  FAKE_PLANE_METRICS="fkst_audit_required_rejections_total fkst_operations_activity_queries_total fkst_operations_sandbox_inventory_requests_total fkst_session_access_registry_generation_state"
  export FAKE_CAN_I FAKE_AUTOMOUNT FAKE_CONFIGMAP_TOKEN FAKE_PVC_PHASE FAKE_STORAGE_CLASS
  export FAKE_CAPACITY FAKE_REPLICAS FAKE_STRATEGY FAKE_RELAY_GRACE FAKE_PLANE_GRACE
  export FAKE_MODE FAKE_RELAY_URL FAKE_FRONTEND_REACHES_RELAY FAKE_SECRET_KEYS
  export FAKE_RECORDS_STARTED FAKE_RECORDS_COMPLETE FAKE_EXTRA_METRIC FAKE_PLANE_METRICS
  export FAKE_RECORDS_STARTED_AFTER FAKE_RECORDS_COMPLETE_AFTER
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

reset_case
expect_success "healthy relay verifies"

reset_case
if "$verifier" >"$output" 2>&1; then
  echo "expected failure: missing context" >&2
  exit 1
fi
echo "ok  a missing context is refused"

reset_case
expect_failure "a restart confirmation must repeat the namespace" --restart-check wrong-namespace

reset_case
FAKE_CAN_I=yes
export FAKE_CAN_I
expect_failure "a relay identity with Kubernetes API access is refused"

reset_case
FAKE_AUTOMOUNT=true
export FAKE_AUTOMOUNT
expect_failure "a mounted ServiceAccount token is refused"

reset_case
FAKE_CONFIGMAP_TOKEN=not-a-real-value
export FAKE_CONFIGMAP_TOKEN
expect_failure "a credential in the relay ConfigMap is refused"

reset_case
FAKE_SECRET_KEYS='FKST_AUDIT_RELAY_WRITE_TOKEN\n'
export FAKE_SECRET_KEYS
expect_failure "a partially populated credential record is refused"

reset_case
FAKE_PVC_PHASE=Pending
export FAKE_PVC_PHASE
expect_failure "an unbound audit volume is refused"

reset_case
FAKE_STRATEGY=RollingUpdate
export FAKE_STRATEGY
expect_failure "a rolling relay update is refused"

reset_case
FAKE_PLANE_GRACE=60
export FAKE_PLANE_GRACE
expect_failure "a disagreeing incomplete grace is refused"

reset_case
FAKE_RELAY_URL=http://somewhere-else
export FAKE_RELAY_URL
expect_failure "a relay URL that does not name the Service is refused"

reset_case
FAKE_FRONTEND_REACHES_RELAY=true
export FAKE_FRONTEND_REACHES_RELAY
expect_failure "a relay reachable from the frontend is refused"

reset_case
# One extra exposition LINE, not a command: the quotes are Prometheus label
# syntax and are meant to be literal.
# shellcheck disable=SC2089
FAKE_EXTRA_METRIC='fkst_audit_relay_records{state="complete",actor_id="1"} 1'
# shellcheck disable=SC2090
export FAKE_EXTRA_METRIC
expect_failure "an identity label in the relay exposition is refused"

reset_case
FAKE_PLANE_METRICS="fkst_audit_required_rejections_total"
export FAKE_PLANE_METRICS
expect_failure "a control plane missing the inventory family is refused"

reset_case
expect_success "a restart that preserves records verifies" --restart-check chronoai-fkst

reset_case
FAKE_RECORDS_COMPLETE_AFTER=0
FAKE_RECORDS_STARTED_AFTER=0
export FAKE_RECORDS_COMPLETE_AFTER FAKE_RECORDS_STARTED_AFTER
expect_failure "records lost across a restart are refused" --restart-check chronoai-fkst

echo "audit relay verifier decisions confirmed"
