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
#
# The fake cluster itself lives in `fake-cluster.sh`, shared with the runbook
# smoke next door.

script_dir=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
# shellcheck source=deploy/kubernetes/tests/fake-cluster.sh
. "$script_dir/fake-cluster.sh"

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
FAKE_FRONTEND_PROBE=reached
export FAKE_FRONTEND_PROBE
expect_failure "a relay reachable from the frontend is refused"

# The isolation probe's AMBIGUOUS outcomes. Each of these used to be reported as
# "blocked", which is the one wrong answer a security control may not give: the
# cage was never tested at all.
for probe in no-tool no-dns exec-error silent; do
  reset_case
  FAKE_FRONTEND_PROBE=$probe
  export FAKE_FRONTEND_PROBE
  expect_failure "an isolation probe that cannot answer ($probe) is not a pass"
done

reset_case
# One extra exposition LINE, not a command: the quotes are Prometheus label
# syntax and are meant to be literal.
# shellcheck disable=SC2089
FAKE_EXTRA_METRIC='fkst_audit_relay_records{state="complete",actor_id="1"} 1'
# shellcheck disable=SC2090
export FAKE_EXTRA_METRIC
expect_failure "an identity label in the relay exposition is refused"

reset_case
FAKE_PLANE_MODE=missing-partial
export FAKE_PLANE_MODE
expect_failure "a control plane missing a reviewed audit family is refused"

reset_case
# Declared but never sampled: the shape a family-name grep accepted, and the
# reason the inventory-independence assertion used to be worthless.
FAKE_PLANE_MODE=headers-only
export FAKE_PLANE_MODE
expect_failure "an inventory family with a header and no samples is refused"

reset_case
FAKE_MAX_RECORDS=0
export FAKE_MAX_RECORDS
expect_failure "a relay publishing no capacity guard is refused"

reset_case
expect_success "a restart that preserves records verifies" --restart-check chronoai-fkst

reset_case
FAKE_RECORDS_COMPLETE_AFTER=0
FAKE_RECORDS_STARTED_AFTER=0
export FAKE_RECORDS_COMPLETE_AFTER FAKE_RECORDS_STARTED_AFTER
expect_failure "records lost across a restart are refused" --restart-check chronoai-fkst

# ---- the outage drill: independence, drain, and PostHog-outage invariants ----

reset_case
expect_failure "an outage confirmation must repeat the namespace" --outage-drill wrong-namespace

reset_case
expect_success "an outage that drains without loss verifies" --outage-drill chronoai-fkst

reset_case
# The independence claim itself: with the relay at zero the control plane must
# still publish live-inventory samples. A deployment whose inventory answer
# depended on the relay would render the header and nothing under it.
FAKE_PLANE_MODE_DEGRADED=headers-only
export FAKE_PLANE_MODE_DEGRADED
expect_failure "live inventory that stops answering without the relay is refused" \
  --outage-drill chronoai-fkst

reset_case
FAKE_RECORDS_COMPLETE_AFTER=0
FAKE_RECORDS_STARTED_AFTER=0
export FAKE_RECORDS_COMPLETE_AFTER FAKE_RECORDS_STARTED_AFTER
expect_failure "records lost across a relay outage are refused" --outage-drill chronoai-fkst

reset_case
FAKE_DEAD_LETTERS_AFTER=3
export FAKE_DEAD_LETTERS_AFTER
expect_failure "an outage that dead-letters instead of retrying is refused" \
  --outage-drill chronoai-fkst

reset_case
FAKE_INGRESS_READY=0
export FAKE_INGRESS_READY
expect_failure "an unreachable PostHog that takes durable ingress down is refused" \
  --outage-drill chronoai-fkst

echo "audit relay verifier decisions confirmed"
