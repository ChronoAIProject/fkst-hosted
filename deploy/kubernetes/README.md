# Kubernetes deployment sources

This directory is the canonical, checked-in source for the FKST namespace
contract. It is deliberately secret-free. A deployment combines these manifests
with non-secret environment patches and credential records whose lifecycle is
outside `chronoai-fkst`.

Every command in this guide pins a Kubernetes context. None changes the current
context, deletes a resource, or contacts a production cluster implicitly.

## Layout

| Path | Contract |
|---|---|
| `base/` | Namespace, Pod Security labels, service accounts, quota/limits, sandbox NetworkPolicy, env-store and Lease RBAC, configuration, workloads, Services, PDBs, Ingress, and the OpenSandbox base template. |
| `external-secrets/` | Provider-neutral External Secrets Operator bindings. It contains remote record names and target key names, never credential values. |
| `overlays/local/` | Disposable local-cluster overlay. Its Kubernetes provider reads source Secrets from `fkst-recovery-source`, outside the namespace being reconstructed. |
| `monitoring/` | Optional Prometheus Operator Services, ServiceMonitors, and fixed-label recovery and activity-trace alerts. It is intentionally separate from the base overlay. |
| `audit-relay/` | Optional durable audit outbox: ServiceAccount, configuration, its own credential binding, ReadWriteOnce claim, hardened single-replica Deployment, ClusterIP Service, PDB, and NetworkPolicy. Separate from `base/` because it owns persistent state. |
| `overlays/required-audit/` | Reference composition that layers `audit-relay/` onto the local overlay and selects `required` delivery. |
| `overlays/local/durable-store/` | Cross-namespace, Secret-only Role and RoleBinding for the encrypted environment store. |
| `overlays/local-migration/` | Temporary local overlay that enables one-time migration from legacy namespace-local profile pairs. |
| `migrations/` | Temporary legacy ConfigMap/Secret read/delete RBAC; never part of the steady overlay. |
| `opensandbox/server-values.yaml` | Canonical FKST tenant, API-key file, and BatchSandbox-template integration for the lifecycle-server chart. |
| `migrate-environment-store.sh` | Convergent legacy migration followed by removal and denial verification of temporary permissions. |
| `restore-namespace.sh` | Ordered, non-destructive namespace convergence. It requires an explicit context and waits for secret materialization before workloads. |
| `verify-namespace.sh` | Redacted live verification of security, RBAC, ExternalSecret, rollout, route, and recovery-readiness contracts. |
| `run-disaster-drill.sh` | Fail-closed kind-only namespace-loss drill with deterministic runtime reconstruction and redacted evidence. |
| `RECOVERY-RUNBOOK.md` | Alert response, escalation, rollback boundaries, recovery objectives, and disaster-drill procedure. |
| `AUDIT-TRACE.md` | Activity-trace reference: architecture, event contract, authorization model, configuration ownership, PostHog prerequisites, capacity worksheet, purge rules, access review. |
| `AUDIT-RUNBOOK.md` | Activity-trace operations: provisioning and smoke tests, the cross-user authorization smoke test, staged rollout and rollback, outage/replay/rotation/backup procedures. |
| `verify-audit-relay.sh` | Redacted live verification of relay least privilege, credential ownership, storage, network isolation, and bounded telemetry. |
| `validate-manifests.sh` | Deterministic render and structural/security policy checks; also runs shellcheck and kubeconform when installed. |

The base runs two control-plane replicas with rolling updates and Kubernetes
Lease election enabled. The stable Lease is
`chronoai-fkst/fkst-control-plane-reconciler`; each Pod injects its unique name
as `FKST_LEADER_IDENTITY`. Exactly one holder owns reconciliation, sweeps,
full resync, runtime mutation, token rotation, health mutation, and validation
cleanup. Every acquisition creates a fresh worker generation and must complete
an immediate full resync.

Both replicas use `/health` for Pod readiness so Deployment convergence is not
blocked by the intentionally idle follower. Public Service routing is a separate
fail-closed contract: the Service selects
`fkst.chronoai.io/leader-serving=true`, which the resync-complete Lease holder
publishes and withdraws through narrow Pod-label RBAC. `/ready` remains `503` on
followers and while acquisition resync or routing publication is incomplete.
Never scale above one with `FKST_LEADER_ELECTION_ENABLED=false`; the render
validator rejects that combination.

## External state contract

The `fkst-external-secrets` SecretStore is environment-owned. A cloud overlay
must patch it to the reviewed provider and bind provider workload identity to
the service account used by that provider. The checked-in local overlay is only
a same-cluster stand-in for disaster testing; it is not a production secret
manager.

The local overlay is also the only overlay that marks `chronoai-fkst` with
`fkst.chronoai.io/disposable=true`. The base namespace deliberately omits that
label. Validation fails if the marker leaks into `base/` or appears on the
durable source namespace.

The provider exposes three logical records:

| Remote record | Materialized Secret | Required key names |
|---|---|---|
| `fkst-control-plane` | `chronoai-fkst/fkst-control-plane-secret` | `FKST_LLM_API_KEY`, `FKST_OSB_EXECD_TOKEN_SEED`, `FKST_GITHUB_APP_ID`, `FKST_GITHUB_APP_PRIVATE_KEY_PEM`, `FKST_GITHUB_APP_SLUG`, `FKST_GITHUB_APP_WEBHOOK_SECRET`, `FKST_GITHUB_OAUTH_CLIENT_SECRET`, `FKST_ENV_STORE_ENCRYPTION_KEY` |
| `fkst-opensandbox-tenant` | `chronoai-fkst/opensandbox-fkst-api-key` and `opensandbox-system/opensandbox-api-key` | `opensandbox-fkst-api-key` |
| `fkst-ingress-tls` | `chronoai-fkst/fkst-ingress-tls` | `tls.crt`, `tls.key` |
| `fkst-audit-relay` | `chronoai-fkst/fkst-audit-relay-secret` | `FKST_AUDIT_RELAY_WRITE_TOKEN`, `FKST_AUDIT_RELAY_READ_TOKEN`, `FKST_POSTHOG_PROJECT_TOKEN`, `FKST_POSTHOG_QUERY_API_KEY` |

The fourth record exists only when the optional `audit-relay/` composition is
applied; its binding travels with that directory rather than with
`external-secrets/`, so a deployment that has not adopted the relay carries no
permanently unresolvable ExternalSecret.

Optional broader-OAuth, log-storage, and activity-trace deployments put
`FKST_GITHUB_BROADER_OAUTH_CLIENT_SECRET`, `FKST_NYXID_CLIENT_SECRET`,
`FKST_POSTHOG_QUERY_API_KEY`, `FKST_AUDIT_RELAY_WRITE_TOKEN`, and
`FKST_AUDIT_RELAY_READ_TOKEN` in the control-plane record. Their non-secret
client IDs, endpoints, bucket names, PostHog host, and numeric project id belong
in the environment ConfigMap patch. ExternalSecret status and Secret key names
are safe to inspect; Secret values are not.

`FKST_POSTHOG_PROJECT_TOKEN` is the PostHog **write** (capture) token. It never
crosses the backend boundary: no frontend build argument, response body, or log
line carries it, and `Debug` output renders it as `<redacted>`. **Which record
holds it is the access boundary.** A relay deployment puts it ONLY in
`fkst-audit-relay` — capture is the relay's job, so a control-plane compromise
can read history but never fabricate it. A deployment capturing directly
(`FKST_POSTHOG_ENABLED=true`, `FKST_AUDIT_DELIVERY_MODE=disabled`) keeps it in
the control-plane record instead. The two shapes are mutually exclusive, and a
deployment using neither omits the key entirely.

`FKST_ENV_STORE_NAMESPACE` selects the namespace-independent profile store. It
persists each profile as one AES-256-GCM encrypted Secret whose data keys are
only `nonce` and `ciphertext`; public metadata includes the content hash and
secret-key inventory needed for redacted recovery verification. The namespace
must remain outside `chronoai-fkst`. The 32-byte standard-base64 encryption key
must remain stable, backed up, and delivered through exactly one of
`FKST_ENV_STORE_ENCRYPTION_KEY` or `FKST_ENV_STORE_ENCRYPTION_KEY_FILE`. Losing
or rotating that key without an explicit re-encryption procedure makes every
existing profile unreadable and fails startup closed.

## Local durable source

Install External Secrets Operator in the disposable local cluster, then create
the three source records in `fkst-recovery-source`. The examples below show the
shape only; use local, untracked input files or literal values and never commit
them:

```bash
helm upgrade --install external-secrets external-secrets \
  --repo https://charts.external-secrets.io \
  --namespace external-secrets --create-namespace \
  --set installCRDs=true \
  --kube-context kind-opensandbox-local

kubectl --context kind-opensandbox-local apply \
  -f deploy/kubernetes/base/namespace.yaml \
  -f deploy/kubernetes/base/service-accounts.yaml

kubectl --context kind-opensandbox-local apply \
  -f deploy/kubernetes/overlays/local/secrets/local-provider.yaml

kubectl --context kind-opensandbox-local --namespace fkst-recovery-source \
  create secret generic fkst-control-plane \
  --from-literal=FKST_LLM_API_KEY='<local value>' \
  --from-literal=FKST_OSB_EXECD_TOKEN_SEED='<local value>' \
  --from-literal=FKST_GITHUB_APP_ID='<local App ID>' \
  --from-file=FKST_GITHUB_APP_PRIVATE_KEY_PEM='<local private-key file>' \
  --from-literal=FKST_GITHUB_APP_SLUG='<local App slug>' \
  --from-literal=FKST_GITHUB_APP_WEBHOOK_SECRET='<local value>' \
  --from-literal=FKST_GITHUB_OAUTH_CLIENT_SECRET='<local value>' \
  --from-literal=FKST_ENV_STORE_ENCRYPTION_KEY='<standard-base64 32-byte key>'

kubectl --context kind-opensandbox-local --namespace fkst-recovery-source \
  create secret generic fkst-opensandbox-tenant \
  --from-literal=opensandbox-fkst-api-key='<local value>'

kubectl --context kind-opensandbox-local --namespace fkst-recovery-source \
  create secret tls fkst-ingress-tls \
  --cert='<local certificate file>' --key='<local key file>'
```

Add a fourth record only when adopting the optional relay. The write and read
tokens must be different values; the relay refuses to start otherwise:

```bash
kubectl --context kind-opensandbox-local --namespace fkst-recovery-source \
  create secret generic fkst-audit-relay \
  --from-literal=FKST_AUDIT_RELAY_WRITE_TOKEN='<local value>' \
  --from-literal=FKST_AUDIT_RELAY_READ_TOKEN='<a DIFFERENT local value>' \
  --from-literal=FKST_POSTHOG_PROJECT_TOKEN='<project capture token>' \
  --from-literal=FKST_POSTHOG_QUERY_API_KEY='<query-read-only key>'
```

In a retained cluster, create/update those source records before deleting a
disposable target namespace. The source namespace is the test's durability
boundary and must not be included in the target deletion.

## Render and restore

Validate the full local overlay without revealing any source Secret:

```bash
deploy/kubernetes/validate-manifests.sh \
  --context kind-opensandbox-local \
  --overlay deploy/kubernetes/overlays/local
```

Restore/converge the namespace in the dependency order from issue #625:

```bash
deploy/kubernetes/restore-namespace.sh \
  --context kind-opensandbox-local \
  --overlay deploy/kubernetes/overlays/local
```

Use `--preflight-only` to ask the target API server to validate the complete
render without changing live resources.

The script applies namespace/security policy, the external durable namespace
and RBAC, identity/RBAC/ExternalSecrets, waits for materialized credentials,
applies services and routes, waits for both rollouts, and finally requires two
healthy control-plane replicas, one durable Lease holder, exactly one matching
Service-selected Pod, and `/ready` from that holder to report a completed
startup resync. It never creates plaintext Secrets, changes kube context, or
performs deletion.

For a pre-I7 installation, run the temporary migration exactly once after the
external encryption key and durable namespace exist:

```bash
deploy/kubernetes/migrate-environment-store.sh \
  --context kind-opensandbox-local
```

The migration overlay grants `fkst-ksa` only `get`, `list`, and `delete` on
legacy ConfigMaps and Secrets. Startup copies and decrypt-verifies each complete
pair before deleting it. A durable record always wins, so retrying after an
interruption is safe. The script then reapplies the steady overlay, removes the
temporary Role and RoleBinding, and proves the old access is denied.

Run live verification independently at any time:

```bash
deploy/kubernetes/verify-namespace.sh --context kind-opensandbox-local
```

After recording only a sentinel's public content-hash annotation and sorted
secret-key annotation, a namespace-loss drill can verify them without reading
values:

```bash
deploy/kubernetes/restore-namespace.sh \
  --context kind-opensandbox-local \
  --sentinel-user-id '<numeric GitHub ID>' \
  --sentinel-name '<normalized profile name>' \
  --sentinel-content-hash '<sha256>' \
  --sentinel-secret-keys 'KEY_ONE,KEY_TWO'
```

`verify-envstore-rbac.sh` remains the focused least-privilege check. It proves
the environment store and validation-pod grants as well as representative
denials:

```bash
deploy/kubernetes/verify-envstore-rbac.sh --context kind-opensandbox-local
```

## Recovery monitoring

The optional monitoring overlay requires the Prometheus Operator CRDs. It adds
a metrics-only Service that selects both control-plane contenders, a
ServiceMonitor, and a PrometheusRule:

```bash
kubectl --context kind-opensandbox-local get customresourcedefinition \
  servicemonitors.monitoring.coreos.com prometheusrules.monitoring.coreos.com
kubectl --context kind-opensandbox-local apply \
  -k deploy/kubernetes/monitoring
```

The control-plane ServiceMonitor drops the two identity-bearing info metrics
before ingestion. Alerts use only fixed namespace/service selectors and bounded
series; their labels and annotations contain no dynamic repository, issue, user,
installation, session, proposal, holder, identity, or credential values. Keep
the overlay optional in clusters without the Operator.

The same overlay carries the activity-trace half: a metrics Service and
ServiceMonitor for the relay, and the `fkst-audit` PrometheusRule covering
required-delivery refusals, relay readiness, delivery backlog, verification lag,
dead letters, incomplete requests, disk pressure, scoped query failures,
inventory failures, and session-visibility readiness. The relay scrape target
carries the distinct `service="fkst-audit-relay-metrics"` label because the
control plane and the relay both publish `fkst_audit_relay_*` families — the
client side and the storage side — and every expression pins which one it means.

For the relay's NetworkPolicy to admit the scraper, label the Prometheus
namespace `fkst.chronoai.io/metrics-scraper=true`. Without it the relay's
metrics port is unreachable, which fails closed.

See [RECOVERY-RUNBOOK.md](RECOVERY-RUNBOOK.md) for the recovery alert-to-response
map, and [AUDIT-RUNBOOK.md](AUDIT-RUNBOOK.md) for the activity-trace one.

## Durable audit relay

Optional, and separate from `base/` because it owns a volume holding the
deployment's undelivered audit trail. It is the workload that makes
`FKST_AUDIT_DELIVERY_MODE=required` possible: a request's start is committed
before its handler runs, so the deployment cannot serve a request it failed to
record.

```bash
deploy/kubernetes/validate-manifests.sh --context kind-opensandbox-local
kubectl --context kind-opensandbox-local apply -k deploy/kubernetes/audit-relay
deploy/kubernetes/verify-audit-relay.sh --context kind-opensandbox-local
```

One replica with a `Recreate` strategy (SQLite has one writer), a ClusterIP
Service and no Ingress, a ReadWriteOnce claim with an explicit size and class, a
ServiceAccount bound to nothing with no mounted token, the restricted Pod
Security profile, probes on unauthenticated internal endpoints with no
credential in any URL, a `minAvailable: 1` PDB, and a NetworkPolicy admitting
only the control plane and a labelled scraper.

Read [AUDIT-TRACE.md](AUDIT-TRACE.md) before provisioning — it carries the
PostHog prerequisites checklist, the capacity worksheet the 20Gi claim and the
disk-pressure thresholds come from, and the purge rules. Then follow
[AUDIT-RUNBOOK.md](AUDIT-RUNBOOK.md) for provisioning, the cross-user
authorization smoke test, and the staged rollout to `required`.

## Disposable recovery drill

The checked-in drill is destructive by design and fail-closed by construction.
It accepts only a non-production `kind-*` context, only the canonical target
namespace with an exact confirmation, and only a target carrying the local
disposable label. The separately labelled durable namespace, one exact test
repository with a prepared live runtime, an encrypted environment sentinel, and
a server-side restore preflight must all pass before the single namespace
delete.

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

The runner invokes `restore-namespace.sh`, waits for the same sorted
deterministic session set, verifies the environment content hash and secret-key
inventory, and records the achieved RTO. JSON and Markdown evidence contain only
bounded state, counts, timestamps, and SHA-256 projections. They never contain
raw context, repository, session, user, issue, Secret value, token, or command
log data. The monthly `.github/workflows/recovery-drill.yml` job runs this exact
script on the protected `fkst-recovery-drill` self-hosted runner and uploads the
redacted artifact even on failure.

## Environment overlays

A deployment overlay should reference `base/` and `external-secrets/` rather
than copy their objects. It must patch, at minimum:

- immutable image references for the control plane and frontend;
- public API/frontend hosts and TLS record;
- LLM endpoint/model and GitHub App bot login/client ID;
- exact issue-scoped `FKST_CROSS_REPO_DELIVERY_GRANTS`, when lifecycle issues
  are explicitly allowed to deliver into another installed repository;
- access model, allowed GitHub logins, and global-admin logins;
- a durable namespace outside `chronoai-fkst`, its Secret-only Role/RoleBinding,
  and `FKST_ENV_STORE_NAMESPACE`;
- optional storage/NyxID endpoints and identities;
- the External Secret provider/store references and remote record identifiers;
- provider-specific Workload Identity annotations;
- runtime class, placement, resources, and replica policy appropriate to the
  environment;
- when the activity trace is adopted: `FKST_AUDIT_DELIVERY_MODE`, the relay's
  PostHog host and numeric project id, `FKST_DEPLOYMENT_ENVIRONMENT`, and the
  audit claim's storage class — see `overlays/required-audit/` for the exact
  patch set and [AUDIT-RUNBOOK.md](AUDIT-RUNBOOK.md#staged-rollout) for the
  order in which to apply it.

Any overlay changing leader timings must preserve
`retry period < renew deadline < lease duration`. The holder cancels its worker
generation after the renew deadline without a confirmed write; another replica
may take over only after the last recorded Lease term expires. Do not grant
Lease `delete`: retained holder, renewal, and transition fields are recovery and
failover evidence.

Keep provider credentials, Secret resources, encryption keys, private keys,
bearer tokens, and encoded secret values outside Git. A production overlay must
bind its durable store to a separately backed-up provider/namespace and define
key custody and recovery. It belongs in the reviewed infrastructure repository
when that environment's ownership boundary is separate from this application
repository.
