#!/bin/sh
set -eu

# Walks the AUDIT-RUNBOOK procedures end to end against a fake cluster.
#
# The issue asks for a runbook smoke covering "initial provisioning, required-mode
# switch, PostHog outage/drain, dead-letter replay, credential rotation, and PVC
# restore in a disposable environment". A runbook nobody executes is a document
# that describes a system it has stopped matching, and every one of these
# procedures ends in a verification step an operator is told to run — so the smoke
# drives each procedure's END STATE through that same verification and requires
# the documented answer.
#
# Two things are asserted for every procedure, and the second is the one that
# makes it a gate rather than a demo:
#
#   1. the state the runbook says the procedure produces VERIFIES;
#   2. the state a HALF-DONE run of it produces is REFUSED.
#
# Without (2) a smoke passes on a verifier that returns success unconditionally.
#
# What this cannot do is prove the runbook's kubectl commands are the right ones
# — that needs a real cluster, and it is the disposable-cluster tier's job
# (`backend/tests/acceptance_integration.rs`). What it does prove is that each
# procedure has a checkable end state, that the checked-in verifier recognises
# it, and that the runbook still documents the procedure at all.

script_dir=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
# shellcheck source=deploy/kubernetes/tests/fake-cluster.sh
. "$script_dir/fake-cluster.sh"

runbook="$source_dir/AUDIT-RUNBOOK.md"

# Each procedure names the runbook heading that owns it. A renamed or deleted
# section fails here, which is what keeps this script and the document in step.
require_section() {
  heading=$1
  grep -q "^## $heading\$" "$runbook" || {
    echo "the runbook no longer documents a section named '$heading'" >&2
    exit 1
  }
}

# ---------------------------------------------------------------- 1. provisioning
require_section "Initial provisioning and smoke test"

reset_case
expect_success "provisioning: a fully provisioned relay verifies"

reset_case
# The provisioning step that is easiest to half-finish: the credential record is
# created, but only some of its keys are populated.
FAKE_SECRET_KEYS='FKST_AUDIT_RELAY_WRITE_TOKEN\nFKST_AUDIT_RELAY_READ_TOKEN\n'
export FAKE_SECRET_KEYS
expect_failure "provisioning: a half-populated credential record is refused"

reset_case
# The claim was requested but never bound — the relay would come up with no
# durable storage at all.
FAKE_PVC_PHASE=Pending
export FAKE_PVC_PHASE
expect_failure "provisioning: an unbound audit volume is refused"

# ------------------------------------------------------- 2. required-mode switch
require_section "Staged rollout"

reset_case
FAKE_MODE=required
export FAKE_MODE
expect_success "required-mode switch: a matched grace and a reachable relay verify"

reset_case
# The switch's specific hazard: the control plane now fails product traffic
# closed on the relay, so a grace the two sides disagree about turns healthy
# requests into incomplete records.
FAKE_MODE=required
FAKE_PLANE_GRACE=60
export FAKE_MODE FAKE_PLANE_GRACE
expect_failure "required-mode switch: a disagreeing incomplete grace is refused"

reset_case
# …and a relay URL that does not name the in-cluster Service means required mode
# would fail closed against something outside the namespace's network policy.
FAKE_MODE=required
FAKE_RELAY_URL=http://somewhere-else
export FAKE_MODE FAKE_RELAY_URL
expect_failure "required-mode switch: a relay URL that is not the Service is refused"

# ----------------------------------------------------- 3. PostHog outage / drain
require_section "PostHog outage"

reset_case
expect_success "posthog outage: an outage that drains without loss verifies" \
  --outage-drill chronoai-fkst

reset_case
FAKE_RECORDS_COMPLETE_AFTER=0
FAKE_RECORDS_STARTED_AFTER=0
export FAKE_RECORDS_COMPLETE_AFTER FAKE_RECORDS_STARTED_AFTER
expect_failure "posthog outage: an outage that loses records is refused" \
  --outage-drill chronoai-fkst

reset_case
# The invariant the runbook leads with: PostHog being unreachable must never take
# durable ingress down with it.
FAKE_INGRESS_READY=0
export FAKE_INGRESS_READY
expect_failure "posthog outage: an outage that stops durable ingress is refused" \
  --outage-drill chronoai-fkst

# --------------------------------------------------------- 4. dead-letter replay
require_section "Replay and dead-letter remediation"

reset_case
# After a successful replay the dead-letter counter is where it was: replay
# re-delivers, it does not manufacture new permanent failures.
FAKE_DEAD_LETTERS=2
FAKE_DEAD_LETTERS_AFTER=2
export FAKE_DEAD_LETTERS FAKE_DEAD_LETTERS_AFTER
expect_success "dead-letter replay: a replay that re-delivers verifies" \
  --outage-drill chronoai-fkst

reset_case
# A replay that dead-letters MORE than it started with is the failure the
# procedure exists to catch: the records are being dropped, not retried.
FAKE_DEAD_LETTERS=2
FAKE_DEAD_LETTERS_AFTER=5
export FAKE_DEAD_LETTERS FAKE_DEAD_LETTERS_AFTER
expect_failure "dead-letter replay: a replay that dead-letters instead is refused" \
  --outage-drill chronoai-fkst

# --------------------------------------------------------- 5. credential rotation
require_section "Key rotation"

reset_case
expect_success "credential rotation: a rotation that keeps every key verifies"

reset_case
# The rotation mistake with the worst blast radius: the new value is pasted into
# the ConfigMap, where it is world-readable to anything that can read the
# namespace's non-secret configuration.
FAKE_CONFIGMAP_TOKEN=rotated-into-the-wrong-object
export FAKE_CONFIGMAP_TOKEN
expect_failure "credential rotation: a credential left in the ConfigMap is refused"

reset_case
# A rotation that dropped a key from the record leaves the relay unable to
# authenticate one half of its conversation.
FAKE_SECRET_KEYS='FKST_AUDIT_RELAY_WRITE_TOKEN\nFKST_POSTHOG_PROJECT_TOKEN\nFKST_POSTHOG_QUERY_API_KEY\n'
export FAKE_SECRET_KEYS
expect_failure "credential rotation: a rotation that dropped a key is refused"

# ---------------------------------------------------------------- 6. PVC restore
require_section "Backup, restore, and retention change"

reset_case
# A restore is only finished when the claim is bound at its documented capacity
# AND the records survive the Pod that mounts it being replaced.
expect_success "pvc restore: a restored volume that survives a restart verifies" \
  --restart-check chronoai-fkst

reset_case
FAKE_RECORDS_COMPLETE_AFTER=0
FAKE_RECORDS_STARTED_AFTER=0
export FAKE_RECORDS_COMPLETE_AFTER FAKE_RECORDS_STARTED_AFTER
expect_failure "pvc restore: a restore whose records vanish on restart is refused" \
  --restart-check chronoai-fkst

reset_case
# The restore that recreated a claim without a usable size: with no reported
# capacity the disk-pressure alert has no denominator, so the deployment would be
# running with its storage headroom alert silently inert.
FAKE_CAPACITY=""
export FAKE_CAPACITY
expect_failure "pvc restore: a claim that reports no capacity is refused"

echo "audit runbook procedures confirmed"
