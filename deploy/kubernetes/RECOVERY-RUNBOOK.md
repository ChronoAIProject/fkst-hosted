# FKST recovery runbook

This runbook is the operator contract for FKST control-plane, runtime, durable
environment-store, leader-election, and namespace recovery. It applies to the
Kubernetes resources under this directory. It does not authorize changes to a
client repository, its issues, or its labels.

Every Kubernetes command must name a reviewed context explicitly. Set the
context once for readability, verify it independently, and retain it on every
command:

```bash
export FKST_CONTEXT='<reviewed-context>'
kubectl --context "$FKST_CONTEXT" config view --minify --output name
```

Never use `kubectl config use-context`. Never delete the leader Lease during an
ordinary incident. Never rotate or replace the environment-store encryption key
as a recovery experiment. Namespace deletion is permitted only through
`run-disaster-drill.sh`, and that runner accepts only an explicitly disposable
`kind-*` target.

## Recovery objectives

| Boundary | P95 objective | Evidence |
|---|---:|---|
| Healthy dependencies to repository discovery | 60 seconds | startup-resync metrics and `/ready` |
| Runtime ready to first worker recovery scan | 30 seconds | runtime health and reconcile telemetry |
| First scan to redrive decision | 60 seconds | typed session recovery projection |
| Combined controller/runtime loss to resumed workflow | 10 minutes | session-set reconstruction evidence |
| Namespace reconstruction after IaC and secrets are available | 30 minutes | drill RTO and post-restore verification |

Escalate when an objective is exceeded, when the durable source is unavailable,
or when the same deterministic session identity appears more than once. Preserve
the Lease transition count, bounded metrics, deployment events, and redacted
drill artifact. Do not attach raw logs, issue bodies, tokens, Secret values,
repository names, user identities, or session identifiers to recovery evidence.

## Initial triage

1. Confirm the context and the two namespace boundaries.

   ```bash
   kubectl --context "$FKST_CONTEXT" get namespace chronoai-fkst fkst-recovery-source \
     -o custom-columns='NAME:.metadata.name,DISPOSABLE:.metadata.labels.fkst\.chronoai\.io/disposable,DURABILITY:.metadata.labels.fkst\.chronoai\.io/durability-boundary'
   ```

2. Inspect rollout, Service publication, and the retained Lease without reading
   Secrets or workload logs.

   ```bash
   kubectl --context "$FKST_CONTEXT" --namespace chronoai-fkst get \
     deployment/fkst-control-plane service/fkst-control-plane \
     lease.coordination.k8s.io/fkst-control-plane-reconciler
   kubectl --context "$FKST_CONTEXT" --namespace chronoai-fkst get pods \
     --selector app.kubernetes.io/name=fkst-control-plane \
     --label-columns fkst.chronoai.io/leader-serving
   ```

3. Read health, readiness, and bounded metrics through an in-cluster caller.

   ```bash
   kubectl --context "$FKST_CONTEXT" --namespace chronoai-fkst exec \
     deployment/fkst-frontend -- wget -qO- http://fkst-control-plane/health
   kubectl --context "$FKST_CONTEXT" --namespace chronoai-fkst exec \
     deployment/fkst-frontend -- wget -qO- http://fkst-control-plane/ready
   kubectl --context "$FKST_CONTEXT" --namespace chronoai-fkst exec \
     deployment/fkst-frontend -- wget -qO- http://fkst-control-plane/metrics
   ```

`/health` proves only process liveness. `/ready` is `200` only for the
resync-complete, Service-published Lease holder. A follower is intentionally
healthy but not ready.

## Scrape missing

Alert: `FKSTControlPlaneScrapeMissing`.

- At two minutes, confirm the metrics Service and both contender endpoints.

  ```bash
  kubectl --context "$FKST_CONTEXT" --namespace chronoai-fkst get \
    service/fkst-control-plane-metrics
  kubectl --context "$FKST_CONTEXT" --namespace chronoai-fkst get endpointslice \
    --selector kubernetes.io/service-name=fkst-control-plane-metrics
  kubectl --context "$FKST_CONTEXT" --namespace chronoai-fkst rollout status \
    deployment/fkst-control-plane --timeout=2m
  ```

- If Pods are healthy and endpoints exist, treat this as a monitoring path
  failure and page the monitoring owner. Do not restart the application.
- If Pods are unavailable, follow the control-plane loss path below. Roll back
  only a known bad workload revision; do not alter the Lease or durable source.

## Startup resync incomplete

Alert: `FKSTStartupResyncIncomplete`.

- The first complete discovery pass should finish within 60 seconds. At five
  minutes, inspect `/ready` and the fixed `fkst_startup_resync_*` series.
- Confirm GitHub App credentials are materialized by Secret key name only:

  ```bash
  kubectl --context "$FKST_CONTEXT" --namespace chronoai-fkst get \
    externalsecret/fkst-control-plane \
    -o custom-columns='READY:.status.conditions[0].status,REASON:.status.conditions[0].reason'
  ```

- A GitHub outage is not a liveness failure. Keep both replicas running and let
  bounded retry/backoff continue. Do not edit, comment on, close, or label an
  issue to force reconciliation.
- Escalate after ten minutes with bounded metrics and provider status. Roll back
  only if the onset correlates with a reviewed application or configuration
  revision.

## Recovery degraded or stale

Alerts: `FKSTRecoveryDegraded`, `FKSTRecoveryStale`.

- `degraded` means the latest full pass or a required reconciler dependency
  failed. `stale` means no successful discovery pass was recorded within the
  combined ten-minute objective.
- Verify the elected replica still renews its Lease and that the startup retry
  counters change. A stable Lease with increasing retry attempts points to a
  dependency failure, not an election failure.
- During a GitHub outage, preserve the live runtime fleet. The durable GitHub
  issue/PR state remains authoritative and will be rediscovered. Do not synthesize
  work by changing client issues.
- Escalate immediately if the condition coincides with duplicate deterministic
  runtime identities or if runtime mutation continues after confirmed Lease
  loss.

## No ready leader

Alert: `FKSTNoReadyLeader`.

1. Compare the Lease transition record to the one Pod carrying the
   leader-serving label.

   ```bash
   kubectl --context "$FKST_CONTEXT" --namespace chronoai-fkst get \
     lease.coordination.k8s.io/fkst-control-plane-reconciler \
     -o custom-columns='TRANSITIONS:.spec.leaseTransitions,RENEW:.spec.renewTime'
   kubectl --context "$FKST_CONTEXT" --namespace chronoai-fkst get pods \
     --selector 'app.kubernetes.io/name=fkst-control-plane,fkst.chronoai.io/leader-serving=true' \
     --output name
   ```

2. Allow one lease duration plus one acquisition resync for failover. With the
   canonical timing this is under two minutes.
3. If no contender acquires, verify Lease and Pod-patch RBAC with
   `verify-namespace.sh`. Do not delete the Lease: its last renewal and transition
   fields are evidence and enforce the takeover boundary.
4. If a known bad rollout caused the incident, use the deployment system to
   restore the last reviewed image/config revision. Re-run namespace verification
   before reopening traffic.

## Leader routing unavailable

Alert: `FKSTLeaderRoutingUnavailable`.

- If the Lease owner is ready but the Service has no endpoint, verify that exactly
  the Lease owner has `fkst.chronoai.io/leader-serving=true` and that `fkst-ksa`
  can patch Pods.
- More than one selected endpoint is a split-routing incident: stop external
  traffic through the environment ingress, preserve all Pods and the Lease, and
  escalate. Do not manually label a contender.
- Zero endpoints during acquisition is fail-closed and expected briefly. Escalate
  after two minutes.

## Lease or routing failures

Alerts: `FKSTLeaderLeaseFailures`, `FKSTLeaderRoutingFailures`.

- Confirm API-server availability and exact RBAC using the canonical verifier:

  ```bash
  deploy/kubernetes/verify-namespace.sh --context "$FKST_CONTEXT" --timeout 5m
  ```

- An isolated optimistic-concurrency conflict can accompany a legitimate
  takeover. Repeated acquire/renew failures or any routing failure require
  escalation.
- Never widen RBAC beyond the checked-in Lease create/get/list/watch/update/patch
  and Pod get/list/patch rules. Never grant Lease delete.

## Excessive leader churn

Alert: `FKSTExcessiveLeaderChurn`.

- More than three acquisitions in thirty minutes is abnormal outside a reviewed
  rollout or node-maintenance window.
- Correlate transition count with node readiness, API-server reachability, and
  rollout history. Do not restart both replicas together.
- If a new revision introduced churn, roll back that revision through the
  deployment system. Preserve the current Lease for review.

## Control-plane loss

Kubernetes recreates lost contenders; startup discovery reconstructs in-memory
state and exactly one new owner publishes routing after resync.

```bash
kubectl --context "$FKST_CONTEXT" --namespace chronoai-fkst rollout status \
  deployment/fkst-control-plane --timeout=5m
deploy/kubernetes/verify-namespace.sh --context "$FKST_CONTEXT" --timeout 5m
```

If the Deployment cannot converge, inspect events and image availability. Do not
replace durable records, issue state, or the Lease to make a rollout appear
healthy. Escalate at five minutes and enforce the ten-minute combined recovery
objective.

## Sandbox loss

The repository ledger and deterministic session identity are authoritative.
After a sandbox disappears, the leader's next scan decides whether pending work
requires recreation. The typed session projection should move through
`recovering/runtime_absent` or `recovering/runtime_starting` and return to
`normal/runtime_live`.

- Confirm the OpenSandbox lifecycle service and controller are healthy with
  context-explicit reads.
- Do not add a work label, comment, close, or reopen a client issue to trigger
  recovery. Wait for the level-triggered scan.
- Escalate if no redrive decision occurs within 60 seconds after the first scan,
  or if two runtimes share one deterministic session identity.

## Environment-store failure

The durable namespace holds encrypted profile records outside the disposable
application namespace. Only `nonce` and `ciphertext` are data keys; public
annotations carry a content hash and secret-key-name inventory.

```bash
deploy/kubernetes/verify-envstore-rbac.sh \
  --context "$FKST_CONTEXT" --durable-namespace fkst-recovery-source
```

- If the source is unavailable, stop namespace reconstruction and escalate to
  the durable-store owner.
- If decryption fails, restore the reviewed encryption key through the external
  secret provider. Never generate a replacement key or edit encrypted records.
- If annotations differ, preserve the record and escalate; do not expose Secret
  values in evidence.

## Namespace loss

For an unplanned loss, confirm IaC, External Secrets, the durable source, and its
encryption key are available, then run the non-destructive canonical restore:

```bash
deploy/kubernetes/restore-namespace.sh \
  --context "$FKST_CONTEXT" \
  --overlay '<reviewed-environment-overlay>' \
  --durable-namespace '<durable-namespace>' \
  --timeout 30m
```

The environment overlay must be reviewed before use. The restore applies
security and identity first, waits for retained credentials, restores the
external profile dependency, deploys workloads, and requires one
resync-complete routed leader. Escalate if the namespace is not verified within
30 minutes. Do not delete a partially restored namespace as a retry mechanism;
the restore is convergent.

## Disposable disaster drill

`run-disaster-drill.sh` is the only reviewed namespace-deletion path. Before its
single delete it requires all of these conditions:

- an explicit `kind-*` context with no production-like marker;
- target `chronoai-fkst` and an exact matching confirmation;
- `fkst.chronoai.io/disposable=true` on that target;
- a different durable namespace carrying
  `fkst.chronoai.io/durability-boundary=external`;
- one exact `OWNER/REPO` and at least one ready, live matching runtime;
- a durable encrypted environment sentinel with a valid content hash;
- a successful server-side dry-run of the canonical local overlay.

Run only after preparing durable work and a live runtime without changing a
client issue as part of the drill:

```bash
deploy/kubernetes/run-disaster-drill.sh \
  --context kind-opensandbox-local \
  --target-namespace chronoai-fkst \
  --confirm-delete chronoai-fkst \
  --durable-namespace fkst-recovery-source \
  --repository '<owner>/<test-repository>' \
  --sentinel-user-id '<numeric-user-id>' \
  --sentinel-name '<normalized-profile-name>' \
  --evidence-dir '<artifact-directory>' \
  --timeout-seconds 1800
```

The runner hashes repository and sorted session identities before writing any
artifact. It verifies the same sorted session set, the environment content hash
and secret-key inventory, two healthy replicas, one routed Lease owner, and the
post-restore transition count. Evidence is emitted as JSON and Markdown even
after a gated failure once the artifact directory is initialized. It contains
no raw context, repository, session, user, issue, Secret value, token, or log.

Run the drill monthly and before material runtime/reconciler changes. The
repository workflow uses the dedicated `fkst-recovery-drill` self-hosted runner
label, a protected disposable-drill environment, non-overlapping concurrency,
and the same script. Scheduled workflows become active only when the workflow is
present on the repository's default branch.
