# FKST activity trace: architecture, configuration, capacity, and retention

This is the reference half of the activity-trace documentation. The operational
half — provisioning, smoke tests, rollout, incidents, rotation, backup — is
[AUDIT-RUNBOOK.md](AUDIT-RUNBOOK.md).

Nothing here contains a real endpoint, project id, token, user, repository, or
session. Every example uses a `<placeholder>`, a loopback address, or an
RFC 2606 `.invalid` host, and it must stay that way: this directory is
distributed publicly.

## What the trace answers

Two different questions, answered by two independent paths:

1. **Historical activity** — who called which backend operation, when it started
   and completed, which explicitly safe arguments were supplied, which identity
   executed the work, and what status came back. The system of record is the
   deployment's own self-hosted PostHog project.
2. **Live sandbox inventory** — which FKST-managed runtimes exist *now*, who
   created them, how old they are, how long they may live, and their normalized
   plus backend-native status. The system of record is Kubernetes or OpenSandbox.

They never share a failure. A PostHog outage cannot hide live sandbox state, and
a runtime-backend outage cannot falsify history.

## Data flow

```text
product request
  -> outer audit middleware            (one terminal record per request)
  -> verified actor + executing principal + typed safe arguments
  -> handler / timeout / leader gate
  -> terminal outcome
  -> AuditSink
       -> required mode: fkst-audit-relay          (SQLite WAL on its own PVC)
       -> PostHog capture/batch API                (complete historical projection)

/operations
  -> authenticated viewer -> effective scope
  -> GET /api/v1/operations/activity
       -> fixed parameterized HogQL, viewer predicate applied AT THE SOURCE
       -> scoped relay read with the same predicate (backlog merge)
       -> merge, deduplicate by event id, keyset page
  -> GET /api/v1/operations/sandboxes
       -> one SessionBackend::list_runtime_inventory()
       -> session-visibility policy against the in-memory access registry
       -> filter, sort, count, serialize only authorized rows
```

The control plane stays **stateless**. Durable buffering lives in the separate
least-privilege relay; the session-access registry is an ephemeral projection
rebuilt from GitHub; PostHog is separately operated.

## Event contract

| Event name | Schema version | Emitted for |
|---|---:|---|
| `fkst api request completed` | 1 | every in-scope request with a terminal HTTP outcome |
| `fkst api request incomplete` | 1 | a request whose process died before a terminal outcome |
| `fkst sandbox lifecycle` | 1 | a runtime create/ready/delete/transition |

`/health`, `/ready`, `/metrics`, `/openapi.json`, and CORS `OPTIONS` are
deliberately excluded to keep probe and scrape noise out of the project.
Everything else — including the GitHub App webhook, OAuth redirects and
callbacks, authentication failures, and the operations polling calls themselves
— is captured.

### Data boundaries

**Recorded:** UTC start/completion timestamps, request and event ids, method,
normalized route template, OpenAPI operation id, status, outcome, stable error
code, duration, numeric GitHub ids, login snapshots, owner/repository names,
issue numbers, session ids, operation flags, numeric limits, package/manifest
references, environment names, branches, and allowlisted engine settings.

**Never recorded:** authorization/cookie/OAuth/PostHog/relay material, raw
bodies, raw or query-bearing URLs, arbitrary headers, stack traces or error
text, environment and secret values, install commands, issue or work-item free
text, and log contents. A malformed payload keeps only bounded content-type and
size metadata. A secret-bearing object exposes counts and allowlisted key names;
values are absent or `[REDACTED]`, never a guessable hash.

Adding a product operation without an explicit argument and redaction policy
fails CI — the generated OpenAPI document is compared against the audit policy.

## Authorization model

Row-level and server-side. The client never filters.

| Record | Regular authenticated user | Global admin |
|---|---|---|
| request with verified `actor_id == viewer.id` | visible | visible |
| request by another human, including a collaborator on the same session | hidden | visible |
| anonymous / unattributed / incomplete with no verified actor | hidden | visible |
| system sandbox lifecycle event | only for an authorized exact session | visible |
| live runtime with trusted session context | when session-visibility passes | visible |
| malformed / orphan / unknown-legacy runtime | hidden, fail closed | visible |

"Belongs to the user" means the record carries a **verified immutable GitHub
numeric id equal to the caller**. Login snapshots, repository membership, shared
session access, request parameters, PostHog `distinct_id`, and frontend state
are never proof of ownership.

A caller may see a session's sandbox and lifecycle rows through any one of:
effective creator (by id, or by login only when an assignee-derived session has
no id), `### Session Collaborators`, `### FKST Contributors` / legacy
`### Log Access Allowlist`, legacy `FKST_LOG_ADMINS`, or `FKST_GLOBAL_ADMINS`.

**Repository administrator or owner status is not one of them.** Neither is
repository visibility, trigger-issue readability, being the trigger author, or
knowing a session id. Runtime annotations are display and correlation data, not
authorization evidence.

Anti-enumeration: an exact `session_id` that does not exist and one the caller
may not see return the same `404`. A regular user selecting the global scope or
a cross-actor filter gets `403` **before** either source is called. While the
session-access projection is cold or recovering, scoped views fail closed with
`503`, never with an apparently complete empty list.

Sharing a session never lets one collaborator read another human's API calls.

## Source of truth

| Question | Authority | Not the authority |
|---|---|---|
| what happened, historically | PostHog project | the relay (a delivery outbox) |
| which runtimes exist now | Kubernetes / OpenSandbox | PostHog |
| who may see a session | the GitHub-derived access registry | runtime annotations, client state |

The relay is an **outbox**, never application storage. Its rows are a delivery
queue plus a short verified overlap used to answer during PostHog lag.

## Delivery guarantee

At-least-once with stable deduplication — never exactly-once:

- the start is committed before the handler runs (`required` mode);
- the completion is committed before the response is released;
- a process that dies after the start is closed as `incomplete`/`aborted` after
  a bounded deadline, with `status_code = null`. No status is ever fabricated;
- complete records batch to PostHog with retries and a deterministic event uuid,
  so a retry is deduplicated rather than duplicated;
- **capture acceptance is not query visibility.** A capture `200` moves a record
  to `posthog_accepted`; only a successful read-back moves it to
  `posthog_verified`;
- recent queued, accepted, and incomplete relay rows are merged into activity
  during PostHog lag, under the same viewer predicate;
- a crash record with no verified actor is global-admin-only, because ownership
  cannot be proven.

## Configuration reference

### Control plane — non-secret (`base/configmap.yaml`)

| Variable | Default | Meaning |
|---|---|---|
| `FKST_AUDIT_DELIVERY_MODE` | `disabled` | `disabled` / `best_effort` / `required` |
| `FKST_AUDIT_RELAY_URL` | relay Service URL | in-cluster base URL; no userinfo |
| `FKST_AUDIT_INCOMPLETE_GRACE_SECS` | `420` | **shared with the relay; must match** |
| `FKST_AUDIT_RELAY_START_TIMEOUT_MS` | `1000` | pre-handler acknowledgement budget |
| `FKST_AUDIT_RELAY_COMPLETION_TIMEOUT_MS` | `5000` | terminal commit budget |
| `FKST_POSTHOG_ENABLED` | `false` | direct capture sink; **mutually exclusive** with a relay delivery mode |
| `FKST_POSTHOG_HOST` | unset | **required by the activity query, not only by capture**; HTTPS outside `test`/`local`; never userinfo |
| `FKST_POSTHOG_PROJECT_ID` | unset | numeric id; not a secret |
| `FKST_POSTHOG_QUERY_TIMEOUT_MS` | `5000` | per-query HTTP budget |
| `FKST_POSTHOG_ACTIVITY_DEFAULT_LIMIT` | `100` | page size when unspecified |
| `FKST_POSTHOG_ACTIVITY_MAX_LIMIT` | `200` | largest accepted page |
| `FKST_POSTHOG_ACTIVITY_MAX_RANGE_DAYS` | `30` | widest single query window |
| `FKST_OPERATIONS_SANDBOX_MAX_RESULT_ITEMS` | `5000` | largest authorized inventory response |
| `FKST_OPERATIONS_SANDBOX_TIMEOUT_MS` | `5000` | budget for the one backend list |
| `FKST_DEPLOYMENT_ENVIRONMENT` | unset | stamped on events; gates plaintext hosts |

Two combinations are refused at startup rather than left to fail quietly:

- **a project id and a query key with no `FKST_POSTHOG_HOST`.** The host is
  shared with capture and easy to omit on a control plane that captures through
  the relay; without it the activity API is disabled and its `503` is
  indistinguishable from an unconfigured key.
- **`FKST_POSTHOG_ENABLED=true` together with a relay delivery mode.** That is
  two capture writers into one project, and it would put
  `FKST_POSTHOG_PROJECT_TOKEN` back into the control-plane record — the exact
  boundary the relay exists to draw.

### Control plane — secret (`fkst-control-plane` record)

| Variable | Why |
|---|---|
| `FKST_POSTHOG_QUERY_API_KEY` | Query-Read-only key for the activity API |
| `FKST_AUDIT_RELAY_WRITE_TOKEN` | writes records to the relay |
| `FKST_AUDIT_RELAY_READ_TOKEN` | reads the relay backlog |

`FKST_POSTHOG_PROJECT_TOKEN` **must not** be in this record when the relay is
deployed. Capture is the relay's job, and keeping the write token out of the
control plane means a control-plane compromise can read history but never
fabricate it.

### Relay — non-secret (`audit-relay/configmap.yaml`)

Bind address, database path, the shared incomplete grace, the PostHog host and
project id, the capacity guard, the delivery-worker and verification cadences,
the two retention windows, and the scoped-read ceilings. Every value is
documented inline in that file with the code's default.

### Relay — secret (`fkst-audit-relay` record)

`FKST_AUDIT_RELAY_WRITE_TOKEN`, `FKST_AUDIT_RELAY_READ_TOKEN`,
`FKST_POSTHOG_PROJECT_TOKEN`, `FKST_POSTHOG_QUERY_API_KEY`. The write and read
tokens must differ; the relay refuses to start otherwise.

None of these ever appears in a ConfigMap, a generated frontend bundle, an
annotation, a command argument, a probe URL, a log line, a metric, or an API
response. `validate-audit-relay.rb` enforces the manifest half of that claim and
`verify-audit-relay.sh` the live half.

`FKST_POSTHOG_HOST` is a ConfigMap value in both processes and is the one place a
credential could re-enter through the front door, so it is validated the same way
everywhere: the relay applies the control plane's rule (no userinfo, TLS unless
the deployment names itself `test`/`local`) rather than a lenient trim, and
`validate-audit-relay.rb` refuses a render that carries a plaintext or
userinfo-bearing host in either ConfigMap.

## Self-hosted PostHog prerequisites

Only PostHog's **public** capture and query APIs are supported: `POST
<host>/capture/`, `POST <host>/batch/`, and `POST
<host>/api/projects/<id>/query/`. Nothing in this milestone reads PostHog's
Kafka, Redis, ClickHouse, or relational schema, and no deployment may introduce
such a coupling — those are internal implementation details that change between
releases and carry no compatibility promise.

### API and version assumptions

The exact contract, and all of it:

| Call | Credential | Used by |
|---|---|---|
| `POST <host>/capture/`, `POST <host>/batch/` | project capture token, in the body | relay delivery |
| `POST <host>/api/projects/<id>/query/` with `{"query":{"kind":"HogQLQuery","query":…}}` | query key, `Authorization: Bearer` | activity API + relay verification |

Responses are consumed by `columns` and `results` only; `hogql`, `types`,
`timings`, `hasMore`, and anything a future release adds are ignored, so a
richer response is compatible by construction.

**No numeric version floor is asserted here, deliberately.** Self-hosted PostHog
ships as dated releases, PostHog documents these endpoints without a version
gate, and a number copied into this file would be wrong for somebody's build and
unverifiable for everybody's. The floor is a **capability**: the deployment's
build must accept the `HogQLQuery` kind on the project query endpoint. Establish
that once, during provisioning (checklist item 4 below), and record the version
you established it against in the deployment's own record — that recorded value,
not a number in this repository, is what a later upgrade is compared against.

A build old enough to predate HogQL rejects the probe with a `4xx` naming the
`kind`, at provisioning time rather than at the first `/operations` page. That
is also why checklist item 3's "Query Read only" scope is qualified with "where
the version supports it": older builds expose only personal API keys, and the
probe is what tells you which kind of identity you are on.

Operator checklist, once per deployment:

- [ ] **1. Dedicated project.** Create or select a project used only by this FKST
      deployment and record its **numeric project id** (it is in the project
      URL). Sharing a project across deployments makes environment separation a
      filter rather than a boundary.
- [ ] **2. Capture token.** Take the project's capture (write) token. It goes in
      the `fkst-audit-relay` record as `FKST_POSTHOG_PROJECT_TOKEN` and nowhere
      else.
- [ ] **3. Query identity, least privilege.** Create a dedicated service account
      or project secret with **Query Read only** where the version supports it;
      otherwise a dedicated, minimum-scope personal API key owned by a service
      identity. **Never a human's general admin key.** It goes in both the
      `fkst-control-plane` record (the activity API) and the `fkst-audit-relay`
      record (verification) as `FKST_POSTHOG_QUERY_API_KEY`.
- [ ] **4. Reachability, TLS, and the HogQL capability.** From the relay and the
      control plane, confirm `<host>/capture/` (or batch capture) and
      `<host>/api/projects/<id>/query/` are reachable over TLS the cluster
      trusts. A plaintext host is refused unless `FKST_DEPLOYMENT_ENVIRONMENT` is
      `test` or `local`, in the control plane and in the relay alike. Then probe
      the one capability this milestone depends on, and record the PostHog
      version it answered from:

      ```bash
      curl -sS -o /dev/null -w '%{http_code}\n' \
        -H "Authorization: Bearer $POSTHOG_QUERY_KEY" \
        -H 'Content-Type: application/json' \
        -d '{"query":{"kind":"HogQLQuery","query":"SELECT 1"}}' \
        '<host>/api/projects/<id>/query/'
      ```

      `200` is the contract. A `4xx` naming the query `kind` means the build
      predates HogQL and must be upgraded before this deployment can read its own
      history; `401`/`403` means the key or its scope, not the version.
- [ ] **5. Retention.** Configure or confirm event retention at least as long as
      the required audit window — **target 90 days** — and write the actual
      configured value into the deployment's own record. PostHog retention is
      external and is never assumed infinite.
- [ ] **6. Clocks.** Verify NTP health on the PostHog host and on every cluster
      node. Timestamps are the trace's primary ordering key, and the shared
      incomplete grace assumes bounded skew.
- [ ] **7. Operator cross-check dashboard.** Save an insight or dashboard
      covering counts, error rates, p95 duration, actors, operations, and
      delivery lag for `fkst api request completed`, `fkst api request
      incomplete`, and `fkst sandbox lifecycle`. This is an operator
      cross-check; the FKST `/operations` UI remains the supported workflow.
- [ ] **8. Backup.** Confirm the PostHog project is covered by the platform's
      existing backup and restore procedure, and that a restore has been
      exercised.

This repository owns no Grafana or dashboard mechanism, so the panels above are
documented rather than checked in. The Prometheus alerts in `monitoring/` are
the checked-in half.

## Capacity worksheet

Fill these in from measurement; the numbers below are the assumptions the
checked-in manifests are sized for. **A deployment that changes any row must
recompute the PVC size, `FKST_AUDIT_RELAY_MAX_RECORDS`, and the disk-pressure
alert thresholds in the same change.**

| Input | Assumed | Where it lands |
|---|---|---|
| peak sustained audited requests | 5 / s | row rate |
| average safe event bytes | ~1.0 KiB | database growth |
| p99 safe event bytes | ~4.0 KiB | `FKST_AUDIT_RELAY_MAX_BODY_BYTES` headroom |
| normal PostHog ingestion lag | < 30 s | `FKST_AUDIT_RELAY_VERIFICATION_DELAY_SECS` |
| relay outage to absorb | 24 h at peak | backlog envelope |
| verified overlap retention | 7 d | `FKST_AUDIT_RELAY_VERIFIED_RETENTION_DAYS` |
| incomplete / dead-letter retention | >= 90 d | `FKST_AUDIT_RELAY_AUDIT_RETENTION_DAYS` |
| PostHog project retention | >= 90 d | external, checklist item 5 |
| activity query concurrency, personal scope | ~1 per active browser tab / 15 s | `FKST_POSTHOG_ACTIVITY_*` |
| activity query concurrency, global-admin scope | a small constant | same |
| sandbox poll concurrency | ~1 per active browser tab / 5 s | `FKST_OPERATIONS_SANDBOX_TIMEOUT_MS` |
| capture batch size | 100 records | `FKST_AUDIT_RELAY_CAPTURE_BATCH_SIZE` |
| verification batch size | 200 event ids | `FKST_AUDIT_RELAY_VERIFICATION_BATCH_SIZE` |
| max accepted record body | 64 KiB | `FKST_AUDIT_RELAY_MAX_BODY_BYTES` |
| writer queue depth | 512 | `FKST_AUDIT_RELAY_WRITER_QUEUE_CAPACITY` |

Derivation for the checked-in 20Gi claim:

```text
rows resident   = 5/s x 86400 x 7d            ~= 3.0 M
row bodies      = 3.0 M x 1.0 KiB             ~= 3.0 GiB
+ index/overhead (~40%)                       ~= 4.2 GiB
+ 24 h outage backlog (432 k rows)            ~= 0.6 GiB
+ WAL, vacuum, and checkpoint headroom        ~= 1.0 GiB
subtotal                                      ~= 5.8 GiB
x 2 safety factor                             ~= 11.6 GiB  -> provision 20Gi
```

`FKST_AUDIT_RELAY_MAX_RECORDS = 5_000_000` is the capacity guard: past it,
ingress is refused with a bounded error and readiness goes false, so required
mode fails closed rather than filling the volume. At the assumed row size that
is roughly 7 GiB — inside the claim with room for the WAL and a vacuum, which
also means **the guard is the ceiling that binds first** in this configuration.
`FKSTAuditRelayCapacityPressure` watches records against it and
`FKSTAuditRelayDiskPressure` watches bytes against the claim, because which one
binds depends on the row size the deployment actually produces.

Derivation for the relay container's requests and limits:

```text
memory
  one capture batch, worst case   100 x 64 KiB (MAX_BODY_BYTES)  ~= 6.4 MiB
  one verification batch          200 event ids + response      ~= 1 MiB
  writer queue at full depth      512 x 64 KiB                  ~= 32 MiB
  SQLite page cache + WAL frames  default cache, one writer     ~= 8 MiB
  tokio + reqwest + TLS + rustls  fixed process floor           ~= 40 MiB
  subtotal                                                      ~= 88 MiB
  request 128Mi (subtotal + slack), limit 512Mi (~4x headroom for a
  vacuum/checkpoint that transiently doubles working set)

cpu
  steady state    5 rows/s of insert + one sweep every 5 s      << 100m
  request 100m (steady state with room for a burst), limit 1 CPU (bounds a
  vacuum/checkpoint or a drain of the 24 h backlog envelope, which is the only
  work here that is CPU-bound at all)
```

A deployment that changes `MAX_BODY_BYTES`, either batch size, or the writer
queue depth recomputes the memory line in the same change; the CPU line follows
the row rate.

Startup validation refuses internally impossible combinations, notably an audit
retention shorter than the verified overlap and any out-of-range budget, so a
worksheet mistake fails the deploy that introduces it.

## Purge rules

These are absolute.

1. **Never automatically purge** a record in `started`, `incomplete`,
   `dead_letter`, or any unverified state. Those are exactly the records whose
   delivery could not be proven, which makes them the last thing an audit trail
   may discard.
2. **Verified records purge only** after both the configured overlap
   (`FKST_AUDIT_RELAY_VERIFIED_RETENTION_DAYS`) has elapsed *and* a successful
   checkpoint has run. Purging inside the overlap would break deduplication and
   the backlog merge.
3. **An emergency purge is an operator action with four preconditions**, all
   required: a backup or export of the affected rows exists; the purge names
   explicit event-id or time bounds; the action is recorded as an audit entry
   with operator, time, reason, and bounds; and the result is verified
   afterwards. There is no broad `rm`, no unauthenticated API, and no
   "delete everything older than X" shortcut. See the runbook's
   [disk pressure](AUDIT-RUNBOOK.md#disk-pressure-and-emergency-purge) section.
4. **PostHog retention is external.** It is configured on the PostHog side,
   reported in this document and the runbook, and surfaced to users as the
   window the UI can answer for. It is never assumed infinite.

## Retention, privacy, and access review

PostHog intentionally holds the **complete** deployment audit dataset. Regular
users never receive PostHog credentials, never query it directly, and never see
another user's rows — every row authorization happens in the FKST backend. That
makes the PostHog project operator a **trusted operational role** whose access
sits outside this product's authorization model and must be governed by the
platform's own access policy.

Review quarterly, and record the outcome:

- [ ] `FKST_GLOBAL_ADMINS` — every login still needs deployment-wide read of all
      users' activity and all runtimes.
- [ ] `FKST_LOG_ADMINS` — the legacy cross-session grant is still intended.
- [ ] `### Session Collaborators` on long-lived sessions — still current people.
- [ ] `### FKST Contributors` / legacy `### Log Access Allowlist` entries.
- [ ] PostHog project members and API keys — least privilege, no human admin key
      in any deployment record.
- [ ] The configured PostHog retention still matches the documented window, and
      the relay's two retention windows still match the capacity worksheet.

## Troubleshooting

| Symptom | Start here |
|---|---|
| product requests returning `503` after enabling required mode | [audit ingress unavailable](AUDIT-RUNBOOK.md#audit-ingress-unavailable) |
| relay Pod not Ready, or PVC unbound | [relay or PVC outage](AUDIT-RUNBOOK.md#relay-or-pvc-outage) |
| activity is minutes behind, backlog growing | [PostHog outage](AUDIT-RUNBOOK.md#posthog-outage) |
| dead-letter alert | [replay and dead-letter remediation](AUDIT-RUNBOOK.md#replay-and-dead-letter-remediation) |
| many `incomplete` records | [incomplete records](AUDIT-RUNBOOK.md#incomplete-records) |
| disk-pressure alert | [disk pressure and emergency purge](AUDIT-RUNBOOK.md#disk-pressure-and-emergency-purge) |
| activity view shows a partial or error state | [activity query failures](AUDIT-RUNBOOK.md#activity-query-failures) |
| sandbox view empty or erroring | [sandbox inventory failures](AUDIT-RUNBOOK.md#sandbox-inventory-failures) |
| ordinary users get `503` on their sandboxes | [session visibility not ready](AUDIT-RUNBOOK.md#session-visibility-not-ready) |
| a user can see somebody else's activity | stop and run the [authorization and isolation smoke test](AUDIT-RUNBOOK.md#authorization-and-isolation-smoke-test) |
