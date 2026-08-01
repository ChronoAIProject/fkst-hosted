# FKST activity-trace runbook

The operator contract for the audit relay, PostHog capture and query, and the
`/operations` surface. The reference half — architecture, configuration,
capacity, retention, PostHog prerequisites — is [AUDIT-TRACE.md](AUDIT-TRACE.md);
namespace, leader-election, and env-store incidents stay in
[RECOVERY-RUNBOOK.md](RECOVERY-RUNBOOK.md).

Every command names a reviewed context. Set it once, keep it on every command,
and never `kubectl config use-context`:

```bash
export FKST_CONTEXT='<reviewed-context>'
kubectl --context "$FKST_CONTEXT" config view --minify --output name
```

Three rules hold everywhere below:

1. **Never print a record, a payload, or a token.** Diagnose from bounded
   metrics, object shapes, counts, and stable error codes. If an incident truly
   needs record content, export it under the
   [replay](#replay-and-dead-letter-remediation) procedure, which is auditable.
2. **Never recreate an empty outbox** to clear an alert. Losing undelivered
   records is a decision, not a cleanup step.
3. **Never rotate a credential by deleting the old one first.** Every rotation
   below has an order that leaves no unrecorded request.

## Alert to response

| Alert | Severity | Go to |
|---|---|---|
| `FKSTAuditIngressUnavailable` | critical | [audit ingress unavailable](#audit-ingress-unavailable) |
| `FKSTAuditRelayNotReady` | critical | [relay or PVC outage](#relay-or-pvc-outage) |
| `FKSTAuditDeadLetters` | critical | [replay and dead-letter remediation](#replay-and-dead-letter-remediation) |
| `FKSTAuditRelayCapacityPressure` | warning / critical | [disk pressure and emergency purge](#disk-pressure-and-emergency-purge) |
| `FKSTAuditRelayDiskPressure` | warning / critical | [disk pressure and emergency purge](#disk-pressure-and-emergency-purge) |
| `FKSTAuditBacklogGrowing` | warning | [PostHog outage](#posthog-outage) |
| `FKSTAuditPostHogUnverified` | warning | [PostHog outage](#posthog-outage) |
| `FKSTAuditIncompleteRequests` | warning | [incomplete records](#incomplete-records) |
| `FKSTOperationsActivityQueryFailures` | warning | [activity query failures](#activity-query-failures) |
| `FKSTSandboxInventoryFailures` | warning | [sandbox inventory failures](#sandbox-inventory-failures) |
| `FKSTSessionVisibilityNotReady` | warning | [session visibility not ready](#session-visibility-not-ready) |

## Initial provisioning and smoke test

Complete the PostHog checklist in [AUDIT-TRACE.md](AUDIT-TRACE.md#self-hosted-posthog-prerequisites)
first. Then:

**1. Create the credential records.** Two records, deliberately separate. Use
your provider's own tooling; the shape below is the local External Secrets
stand-in, and the values are never committed:

```bash
kubectl --context "$FKST_CONTEXT" --namespace fkst-recovery-source \
  create secret generic fkst-audit-relay \
  --from-literal=FKST_AUDIT_RELAY_WRITE_TOKEN='<generated value>' \
  --from-literal=FKST_AUDIT_RELAY_READ_TOKEN='<a DIFFERENT generated value>' \
  --from-literal=FKST_POSTHOG_PROJECT_TOKEN='<project capture token>' \
  --from-literal=FKST_POSTHOG_QUERY_API_KEY='<query-read-only key>'
```

Add `FKST_AUDIT_RELAY_WRITE_TOKEN`, `FKST_AUDIT_RELAY_READ_TOKEN`, and
`FKST_POSTHOG_QUERY_API_KEY` — the same three values — to the existing
`fkst-control-plane` record. Do **not** add `FKST_POSTHOG_PROJECT_TOKEN` there.

**2. Set the non-secret configuration.** In the environment overlay, in **both**
ConfigMaps:

| ConfigMap | Keys |
|---|---|
| `fkst-audit-relay-config` | `FKST_POSTHOG_HOST`, `FKST_POSTHOG_PROJECT_ID`, `FKST_DEPLOYMENT_ENVIRONMENT` |
| `fkst-control-plane-config` | `FKST_POSTHOG_HOST`, `FKST_POSTHOG_PROJECT_ID`, `FKST_DEPLOYMENT_ENVIRONMENT` |

plus the PVC's storage class. Leave `FKST_AUDIT_DELIVERY_MODE` at `disabled` for
now.

**The control plane's `FKST_POSTHOG_HOST` is the one people leave off**, because
this control plane does not capture — the relay does. But the activity query
reads the *same* host, so without it `/operations` is permanently unconfigured
and answers `503`. The control plane refuses to boot on a project id plus a
query key with no host, so a missed host is a crash-loop naming the variable
rather than a silently dead read path. `overlays/required-audit/` shows the
whole set. Leave `FKST_POSTHOG_ENABLED` false: with the relay capturing, a
control plane that also captured would be a second writer into the same project,
and that combination is refused at startup too.

**3. Render, validate, then apply the relay.** The validator contacts no cluster
— it renders, diffs, and lints local files — so `--context` is optional and the
whole check runs on a laptop or a CI runner with no kubeconfig, before anything
reaches a cluster.

```bash
deploy/kubernetes/validate-manifests.sh --context "$FKST_CONTEXT"
kubectl --context "$FKST_CONTEXT" apply -k deploy/kubernetes/audit-relay
kubectl --context "$FKST_CONTEXT" --namespace chronoai-fkst rollout status \
  deployment/fkst-audit-relay --timeout=5m
```

**4. Verify the relay independently of anything else.** Expect eight objects,
five `no` least-privilege answers, four credential key names present and none in
the ConfigMap, a `Bound` claim with an explicit class and capacity, one
`Recreate` replica, agreeing grace values, durable ingress ready, the frontend
resolving the relay and being refused at the port, and eight bounded relay metric
families — including a non-zero `fkst_audit_relay_max_records`, without which the
headroom alert cannot evaluate — with no identity token.

```bash
deploy/kubernetes/verify-audit-relay.sh --context "$FKST_CONTEXT"
```

**In a disposable cluster, add the two drills.** They mutate the relay
Deployment and briefly stop durable ingress, so each repeats the namespace as
confirmation, and neither belongs on a deployment serving required-mode traffic:

```bash
deploy/kubernetes/verify-audit-relay.sh --context "$FKST_CONTEXT" \
  --restart-check chronoai-fkst --outage-drill chronoai-fkst
```

`--restart-check` proves the PVC survives a roll. `--outage-drill` scales the
relay to zero and back, and asserts the three behavioural claims a healthy
cluster cannot show you: that live sandbox inventory keeps publishing while the
relay is gone, that the outage drains without losing a record or dead-lettering
one, and that a PostHog it cannot reach never takes durable ingress down.

**5. Verify capture, query, and inventory separately.** They fail for different
reasons and must be proved one at a time.

- *Capture:* `fkst_audit_relay_capture_total{result="accepted"}` increases while
  `{result="permanent"}` stays flat.
- *Query/verification:* `fkst_audit_relay_verification_total{result="verified"}`
  increases. If it stays zero while capture succeeds, the query key or project
  id is wrong — capture and query use different credentials.
- *Inventory:* `GET /api/v1/operations/sandboxes` answers even with the relay
  scaled to zero. That independence is a requirement, not an accident, and
  `--outage-drill` above is what checks it rather than asserting it.
- *Activity:* `GET /api/v1/operations/activity` must not answer
  `503 audit_query_not_configured`. If it does, the control plane is missing one
  of `FKST_POSTHOG_HOST` / `FKST_POSTHOG_PROJECT_ID` /
  `FKST_POSTHOG_QUERY_API_KEY` — and a missing host would have crash-looped the
  Pod, so check the other two first.

**6. Trace one harmless request end to end.** Issue an authenticated request
with a request id you choose, then follow it through all three layers:

```bash
curl -sS -H "Authorization: Bearer <your GitHub token>" \
     -H "X-Request-Id: <your chosen request id>" \
     '<public base url>/api/v1/operations/activity?limit=1' -o /dev/null -w '%{http_code}\n'
```

- **relay:** the record exists (its state moves `started` -> `complete` ->
  `posthog_accepted` -> `posthog_verified`);
- **PostHog:** the saved insight shows the `fkst api request completed` event;
- **API/UI:** the same request id appears in `/operations` under **My activity**
  when queried by that id.

Seeing it in all three, in that order, is what "the trace works" means. Seeing it
only in PostHog means verification is not configured; only in the relay means
capture is failing.

**7. Prove a live runtime appears** from every backend the deployment uses
(Kubernetes and/or OpenSandbox): start one session, confirm it appears under
**My accessible sandboxes** with a creator, an age, and a normalized status.

## Authorization and isolation smoke test

Run this after provisioning, after any change to access configuration, and
whenever someone reports seeing data they should not. It needs four identities
and one session:

| Fixture | Setup |
|---|---|
| **User A** | creates the trigger issue; effective creator of session S |
| **User B** | listed in session S's `### Session Collaborators` |
| **User C** | unrelated; may even be a repository **admin** — that grants nothing here |
| **Admin G** | listed in `FKST_GLOBAL_ADMINS` |

Each of A, B, C issues one harmless authenticated request and notes its request
id. Then check every row:

- [ ] **A finds only A's request** by request id; **B finds only B's**; **C finds
      only C's**. Searching for another user's request id returns nothing — not
      an error, nothing.
- [ ] **A and B both see session S's sandbox** and its lifecycle timeline.
- [ ] **Neither A nor B sees the other's API-request rows**, in the shared
      session's timeline or anywhere else. This is the single most important row
      in this table: a shared session is not shared activity.
- [ ] **C cannot see the sandbox.** Requesting S's exact `session_id` returns
      `404` — the same answer as a `session_id` that does not exist. Repository
      admin rights do not change this.
- [ ] **G sees A's and B's requests**, an unattributed test event with no
      verified actor, and a deliberately malformed managed-runtime fixture
      (attribution `unknown_legacy`), which no regular user may see.
- [ ] **A crafted request is refused before any source is queried.** As user A:
      `scope=all` -> `403 operations_scope_forbidden`; `actor_id=<B's id>` ->
      `403`; a cursor issued to another viewer or another scope -> rejected.
      Confirm `fkst_operations_activity_scope_rejections_total` increments while
      `fkst_operations_activity_queries_total{result="success"}` does **not** —
      that is the proof no source query was issued.
- [ ] **Cold start fails closed.** Restart the control plane (or force a full
      resync) and, before the projection is published, confirm a regular user's
      sandbox request returns `503 session_visibility_unavailable` and the UI
      says *recovering* — never an empty "you have no sandboxes". Then confirm it
      becomes complete once the atomic access generation is ready.

Any row that fails is a `AUTH-0x` regression: stop the rollout, do not "fix" it
in the frontend, and treat it as a security incident.

## Staged rollout

Never go straight to `required`.

**Stage 1 — shadow.** Deploy the relay and set
`FKST_AUDIT_DELIVERY_MODE: best_effort`. Product responses are never affected;
relay failures are counted and logged. Record the configuration revision you
deployed.

**Stage 2 — observe for a defined window** (at least one full business day, and
one peak):

- [ ] a redaction canary review: sample the PostHog project and confirm no raw
      body, URL, header, token, or free text ever appears;
- [ ] volume matches the capacity worksheet's assumed rate within an order of
      magnitude;
- [ ] `fkst_audit_relay_oldest_record_age_seconds{state="complete"}` stays under
      the expected ingestion lag;
- [ ] deduplication holds: retried captures do not create duplicate logical
      events in PostHog;
- [ ] `fkst_audit_relay_db_bytes` growth extrapolates to well inside the claim.

**Stage 3 — canary `required`.** If the topology supports splitting traffic or
replicas, put one replica or slice on `required` first and watch
`fkst_audit_required_rejections_total` for a full peak.

**Stage 4 — fleet `required`**, only after relay readiness and every alert have
been stable for the observation window.

**Rollback triggers.** Any one of these, immediately:

- `FKSTAuditIngressUnavailable` fires and is not a transient rollout artefact;
- `FKSTAuditRelayNotReady` fires and the relay cannot be restored within the
  deployment's error budget;
- p99 request latency grows by more than the start-plus-completion budget;
- disk pressure reaches critical with no immediate remediation.

**Rollback is `required` -> `best_effort`, and nothing else.** Patch the mode in
the environment overlay, apply, and restart the control plane. It is an explicit
operator incident action: raise a visible alert or status banner for its
duration, record who did it and why, and treat the deployment as
**unaudited-tolerant** until reverted. There is no automatic silent fallback.

## Audit ingress unavailable

Alert: `FKSTAuditIngressUnavailable`. Required mode refused product traffic.

1. Split the cause by reason label — `audit_ingress_unavailable` and
   `audit_completion_unconfirmed` mean the relay could not answer;
   `audit_ingress_conflict` and `audit_completion_conflict` mean it answered and
   holds different content for that event id (an id collision, or a request that
   outlived its completion deadline and was already closed as `incomplete`).
2. Outage reasons: go to [relay or PVC outage](#relay-or-pvc-outage).
3. Conflict reasons: check `FKST_AUDIT_INCOMPLETE_GRACE_SECS` in **both**
   ConfigMaps against `FKST_ENV_VALIDATE_DEADLINE_SECS + 60 + 30`. A grace that
   is too short force-closes still-running requests, which is exactly what a
   completion conflict looks like. `verify-audit-relay.sh` checks the two values
   agree; it cannot check they are large enough — that is this step.
4. If the deployment is failing closed and cannot be fixed inside the error
   budget, roll back to `best_effort` under the [staged rollout](#staged-rollout)
   rules.

## PostHog outage

Alerts: `FKSTAuditBacklogGrowing`, `FKSTAuditPostHogUnverified`.

1. **Classify the failure** — capture (`fkst_audit_relay_capture_total` with
   `result="retryable"`/`"permanent"` rising), query
   (`fkst_operations_activity_queries_total` with
   `result="upstream_error"`/`"unavailable"`), or verification
   (`fkst_audit_relay_verification_total{result="failed"}` while capture is
   accepted). Capture and verification use **different credentials**; only one
   being broken usually means a key or project-id problem, not an outage.
2. **Confirm the relay is still writable.** `fkst_audit_relay_ingress_ready` is
   `1`. A PostHog outage must not make it `0` — an outbox whose destination is
   down is doing its job. If it is `0`, this is a
   [relay outage](#relay-or-pvc-outage), not a PostHog one.
3. **Confirm users see the truth.** `/operations` reports a partial/delayed
   state and merges recent relay rows under the same viewer predicate. It must
   never show an apparently complete empty result.
4. **Calculate remaining runway** from `fkst_audit_relay_db_bytes` growth against
   the claim, and from the record count against
   `FKST_AUDIT_RELAY_MAX_RECORDS`. Escalate **before** either is exhausted, not
   after: at the guard, ingress is refused and required mode fails closed.
5. **After PostHog returns**, watch the backlog drain and the accepted rows move
   to verified. Confirm no duplicate logical events — capture is deduplicated on
   a deterministic uuid, so a drained retry storm must not double any event.

## Relay or PVC outage

Alert: `FKSTAuditRelayNotReady`. Covers an unreachable relay, an unbound or
failed claim, and a corrupted database or migration.

**Expected product behaviour first:** in `required` mode, product traffic fails
closed with `503`. That is the design — the deployment refuses to serve a request
it cannot record. Say so in the incident channel before anyone "fixes" it by
disabling the audit.

1. **Inspect without dumping anything.**

   ```bash
   kubectl --context "$FKST_CONTEXT" --namespace chronoai-fkst get \
     deployment/fkst-audit-relay pvc/fkst-audit-relay-data
   kubectl --context "$FKST_CONTEXT" --namespace chronoai-fkst describe \
     pod --selector app.kubernetes.io/name=fkst-audit-relay | tail -30
   deploy/kubernetes/verify-audit-relay.sh --context "$FKST_CONTEXT"
   ```

   Read events, readiness, and claim status. Do **not** read Pod logs looking for
   event JSON, and never `kubectl exec … cat` the database.

2. **Unbound claim** — the storage class is missing, exhausted, or zone-pinned.
   Fix the class; never "temporarily" swap the volume for an `emptyDir`.
3. **Corruption or a failed migration** — restore from the most recent snapshot
   or attach the recovered volume, then start the relay and confirm the migration
   completes, readiness returns, the record gauges are plausible against the
   pre-incident values, and delivery resumes.
4. **Never recreate an empty database to clear the alert.** If the volume is
   genuinely unrecoverable, record an explicit loss acknowledgement — who
   approved it, the time window lost, and the estimated record count — before
   provisioning a new claim.

**Planned node maintenance.** The PDB (`minAvailable: 1`) intentionally blocks an
ordinary drain of the single replica. Do not delete the PDB. Instead: announce a
short window, confirm no `required`-mode traffic is critical for it, roll the Pod
deliberately with `kubectl rollout restart` once the node is cordoned and the
scheduler can place it elsewhere, and confirm readiness before uncordoning.

## Incomplete records

Alert: `FKSTAuditIncompleteRequests`.

An `incomplete` record is honest: the process died before a terminal outcome, so
`status_code` is `null` and no status is invented. A trickle during a rollout is
normal. A sustained rate means Pods are being killed mid-request (check
`terminationGracePeriodSeconds`, the preStop hook, and eviction events), the
shared grace is too short (step 3 of
[audit ingress unavailable](#audit-ingress-unavailable)), or a handler genuinely
hangs past its budget — a product bug the audit trail just surfaced.

Incomplete records are **never** auto-purged and are global-admin-only when they
carry no verified actor.

## Disk pressure and emergency purge

Alerts: `FKSTAuditRelayCapacityPressure` (70% / 85% of
`FKST_AUDIT_RELAY_MAX_RECORDS`) and `FKSTAuditRelayDiskPressure` (70% / 85% of
the claim).

**Read which one fired first — they are different ceilings.** The record guard
refuses ingress at a row count; the volume refuses writes at a byte count. At the
worksheet's assumed ~1 KiB row the guard binds first, at roughly a third of the
claim, so `FKSTAuditRelayCapacityPressure` is normally the alert that arrives.
`FKSTAuditRelayDiskPressure` arriving first means rows are fatter than the
worksheet assumes — recheck the average event size before resizing anything, or
the new numbers will be wrong the same way.

Reaching either ceiling is not a degradation: at the record guard ingress is
refused, readiness drops, and required mode fails every product request closed.

**Preferred remedies, in order:**

1. Fix delivery. Pressure almost always means a backlog that is not draining —
   go to [PostHog outage](#posthog-outage).
2. Grow the volume, if the storage class allows expansion. Recompute
   `FKST_AUDIT_RELAY_MAX_RECORDS` and the byte thresholds in the same change;
   the record thresholds are ratios against the published guard and follow it
   automatically.
3. Shorten `FKST_AUDIT_RELAY_VERIFIED_RETENTION_DAYS` — but see
   [retention change](#backup-restore-and-retention-change): it also shortens the
   deduplication and backlog-merge window.

**Emergency purge is the last resort and requires all four preconditions:**

- [ ] a backup or export of the affected rows exists and has been checked;
- [ ] the purge names **explicit bounds** — event ids, or a closed time range;
- [ ] an audit entry records operator, time, reason, and bounds;
- [ ] the result is verified afterwards.

Only verified rows may be purged this way. `started`, `incomplete`, unverified,
and `dead_letter` rows are never eligible. There is no broad `rm`, no
unauthenticated API, and no "delete everything older than X" shortcut: the relay
itself only ever deletes verified rows past the overlap, and an operator must not
do more than the relay would.

Because SQLite has one writer, any direct maintenance requires scaling the relay
to zero first, attaching the claim to a short-lived maintenance Pod, doing the
bounded work, and scaling back. Ingress is unavailable for that whole window —
in `required` mode that is a product outage, so schedule it.

## Replay and dead-letter remediation

Alert: `FKSTAuditDeadLetters`. A dead letter will never reach PostHog without an
operator.

1. **List counts and stable codes, not payloads.** Start from
   `fkst_audit_relay_dead_letters_total{reason}` — `permanent` (PostHog refused:
   auth or schema), `attempts_exhausted` (retries ran out while still retryable),
   `invalid` (the stored body could not be projected). The relay's structured
   logs carry the same bounded codes; export no record content at this step.
2. **Fix the cause.** `permanent` is nearly always a wrong capture token,
   project id, or host; `attempts_exhausted` is a long outage; `invalid` is a
   contract bug worth an issue.
3. **Export only if the incident needs it.** Use the scoped read endpoint with
   the READ token, from inside the cluster, bounded by time and page size — never
   the whole table, never to a shared location, and encrypted at rest:

   ```text
   GET /internal/v1/audit/records?scope=all&record_kind=all&from=<iso>&to=<iso>&limit=<n>
   Authorization: Bearer <relay read token>
   ```

4. **Requeue idempotently.** There is deliberately no requeue API: rewriting an
   audit record's delivery state is an operator action, not an HTTP route. Scale
   the relay to zero, attach the claim to a maintenance Pod, and move exactly the
   selected rows back to `complete` bounded by event id or a closed time range —
   never a bare `UPDATE audit_records SET state='complete'`. Re-capture is safe:
   the event uuid is deterministic, so PostHog deduplicates.
5. **Verify and clear.** Scale back up, confirm the rows reach
   `posthog_verified`, confirm they are visible in PostHog and in
   `/operations`, and confirm the alert clears on its own.
6. **Retain the evidence:** operator, time, reason, the bounds used, and the
   counts before and after.

## Key rotation

Four credentials, rotated independently. Each order below leaves **no unaudited
window**, which matters more than convenience: the relay accepts exactly one
write and one read token at a time, compared in constant time, and its metrics
never reveal which key matched.

**PostHog project (capture) token.** Capture is the relay's only writer.

1. Create the new token in PostHog, keeping the old one valid.
2. Update `FKST_POSTHOG_PROJECT_TOKEN` in the `fkst-audit-relay` record.
3. Wait for the ExternalSecret refresh, then restart the relay.
4. Confirm `fkst_audit_relay_capture_total{result="accepted"}` resumes.
5. **Only then** revoke the old token in PostHog.

Queued records are never lost during this: they stay committed in the outbox and
are captured with whichever token is live when the sweep runs.

**PostHog query key.** Used by two consumers — the control plane's activity API
and the relay's verification.

1. Create the new key with Query-Read-only scope.
2. Update `FKST_POSTHOG_QUERY_API_KEY` in **both** records.
3. Restart the relay and the control plane.
4. Confirm verification and activity queries succeed.
5. Revoke the old key.

**Relay write token.** The relay accepts exactly one write token, so there is no
"both keys valid" window to lean on. Two supported orders, neither of which lets
a request run unrecorded:

- *Coordinated restart (default).* Update both records, restart the relay, then
  restart the control plane. In between, required mode answers `503` — the
  deployment is briefly **unavailable**, never **unaudited**, which is the whole
  point of failing closed. Schedule and announce it; never bridge the gap by
  dropping to `best_effort` without recording it as a
  [rollback](#staged-rollout).
- *Blue/green (zero downtime).* Deploy a second relay Deployment, Service, and
  claim carrying the new token, point `FKST_AUDIT_RELAY_URL` at it, restart the
  control plane, then keep the old relay running until its backlog has drained
  and verified before removing it. Costs a second volume; removes the window.

**Relay read token.** Only the backlog merge uses it, so rotation is safe at any
time: update both records, restart the relay, restart the control plane. During
the gap, activity falls back to PostHog-only and marks itself partial.

**After every rotation:** confirm the old credential is rejected, confirm no key
appears in any log line, ConfigMap, render, or git history, and record the
rotation date.

## Backup, restore, and retention change

**Relay PVC snapshots.** Snapshot at least daily, and always before a migration,
a retention change, or maintenance touching the volume. Retain snapshots at
least as long as `FKST_AUDIT_RELAY_AUDIT_RETENTION_DAYS`, and bind the claim to a
storage class with `reclaimPolicy: Retain`.

**Restore drill, quarterly.** Restore a snapshot into a disposable cluster, start
a relay against it, confirm the migration completes and readiness returns, and
confirm a scoped read answers. Record the achieved time — a backup that has never
been restored is not a backup.

**The relay overlap and PostHog backups are not substitutes.** The relay holds
days; PostHog holds the configured window (target >= 90 days). Losing PostHog
loses history the relay cannot reconstruct; losing the relay loses records
PostHog never received. Both need backup coverage.

**Changing retention.**

- *Increasing* the relay's verified overlap: recompute the capacity worksheet,
  grow the claim and `FKST_AUDIT_RELAY_MAX_RECORDS` and the alert thresholds
  first, then raise the value.
- *Decreasing* it: understand that the deduplication and backlog-merge window
  shrinks with it. Never set it below the worst PostHog outage you intend to
  survive.
- `FKST_AUDIT_RELAY_AUDIT_RETENTION_DAYS` must stay at least the verified
  window; startup refuses an inverted pair.
- *PostHog retention* is changed in PostHog, and the new value must be written
  into [AUDIT-TRACE.md](AUDIT-TRACE.md#capacity-worksheet)'s worksheet — users
  are told what window `/operations` can answer for, so it may not drift.

## Activity query failures

Alert: `FKSTOperationsActivityQueryFailures`. History is unreadable or partial;
live sandbox state is unaffected and must not be treated as an outage.

1. Distinguish a *query* failure (PostHog unreachable, wrong project id,
   rejected key) from a *partial page* (a source answered, incompletely).
2. Check the query credential and project id — the activity API uses the
   Query-Read-only key, never the capture token. With
   `FKST_AUDIT_RELAY_READ_TOKEN` unset or wrong, the backlog merge degrades to
   PostHog-only and pages are marked partial.
3. `403`s here are not failures. A regular user selecting the global scope or a
   cross-actor filter is refused before any source is called, and that shows up
   in `fkst_operations_activity_scope_rejections_total`, not in this alert.

## Sandbox inventory failures

Alert: `FKSTSandboxInventoryFailures`. The runtime backend cannot answer.

1. Identify the backend from the `backend` label (`kubernetes`, `opensandbox`,
   or `none`) and check that backend's own health — Kubernetes API reachability
   or the OpenSandbox lifecycle server.
2. `too_large` is different: the authorized result exceeded
   `FKST_OPERATIONS_SANDBOX_MAX_RESULT_ITEMS`. Narrow the filters or raise the
   ceiling deliberately.
3. This path never touches PostHog. If it is failing at the same time as an
   audit alert, they are two incidents, not one.

## Session visibility not ready

Alert: `FKSTSessionVisibilityNotReady`. The GitHub-derived access projection has
stayed cold or recovering, so regular users' scoped sandbox views fail closed
with `503` rather than showing a misleading empty list.

1. Seconds after a restart this is expected. Ten minutes is not.
2. The projection is rebuilt by the reconciler's full resync — check
   `fkst_startup_resync_complete` and the leader state in
   [RECOVERY-RUNBOOK.md](RECOVERY-RUNBOOK.md#startup-resync-incomplete). This
   alert is usually a symptom of a discovery problem, not its own fault.
3. Global admins are unaffected, so their raw inventory is a useful way to
   confirm the runtime backend is healthy while the projection is not.
4. Never "fix" this by widening visibility. Failing closed is the requirement.
