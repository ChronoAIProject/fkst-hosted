# fkst-control-plane on Kubernetes — SAMPLE manifests

Per-deployable **sample** Kubernetes objects for the **control-plane** (the
public REST API). Every value here is an EXAMPLE — the config loaders
(`backend/fkst-control-plane/src/config.rs` and its `reconcile/` + `github_app/`
siblings, plus the linked `backend/fkst-engine/src/config.rs`) are the canonical
source of truth for the env contract. The worker's objects live in
[`backend/fkst-worker/k8s_sample/`](../../fkst-worker/k8s_sample/README.md).

The controller is **datastore-free** (#143): there is no MongoDB to deploy. It
is a **single authoritative replica** whose claim authority is an in-memory,
single-writer map; on restart it rebuilds that state from the live workers'
self-reports and the claimed goal-issue labels (never a datastore). The **durable
audit trail is GitHub Issues**; the **live ephemeral state** is observable at
`GET /api/v1/admin/state` (admin-gated) and `GET /metrics` (Prometheus). The
**worker fleet** — not the controller — scales horizontally (its HPA lives in the
worker dir).

Primary target is **Docker Desktop** (context `docker-desktop`); the portability
notes cover kind/minikube/k3d and real registries.

## What this directory ships

Everything lands in the **`fkst-hosted`** Namespace (shared by both deployables;
declared here, not duplicated in the worker dir):

| Object | Kind | Purpose |
|--------|------|---------|
| `fkst-hosted` | Namespace | Shared namespace for both deployables |
| `fkst-control-plane` | Deployment (**1 replica**, `Recreate`) | The Rust control-plane (single authoritative controller, see "Single-replica") |
| `fkst-control-plane` | PodDisruptionBudget (`maxUnavailable: 1`) | Lets the single controller pod be drained/recycled during voluntary disruptions |
| `fkst-hosted` | Service (ClusterIP, `80 → 8080`, **no Ingress**) | Cluster-internal entry to the control-plane (proxy-trust isolation) |
| `fkst-control-plane-config` | ConfigMap | Comprehensive non-secret runtime config |
| `fkst-control-plane-secret` | Secret | **Created by you, before apply** (template: `secret.example.yaml`) |

There is **no MongoDB object** — the controller is datastore-free (#143).
`secret.example.yaml` is a **template only** (placeholders, not in
`kustomization.yaml`) and is never a real Secret.

### No Ingress — proxy-trust isolation

The Service is **ClusterIP only**; there is **no Ingress**. fkst-hosted trusts
the NyxID-proxy-injected identity (issue #113) precisely because it is reachable
ONLY via the in-cluster NyxID proxy, never directly by a public client. Local
access is `kubectl -n fkst-hosted port-forward svc/fkst-hosted 8080:80`.

### Image-baked ENV (not set by any manifest)

`FKST_HOSTED_PORT=8080`, `FKST_HOSTED_BIND_ADDR=0.0.0.0` are baked into the image
by the Dockerfile; `FKST_RUNTIME_ROOT=/var/lib/fkst/runtime` is baked into the
engine-laden image the control-plane transitionally runs from.
`FKST_POD_ID` is downward-API only (`fieldRef: metadata.name`; loader fallback
`FKST_POD_ID → HOSTNAME → local-<uuid>`). `enableServiceLinks: false` is REQUIRED
— the Service is named `fkst-hosted`, so Docker-link injection would set
`FKST_HOSTED_PORT=tcp://<clusterIP>:80`, colliding with the `FKST_HOSTED_*`
prefix and failing the fail-closed loader at startup.

## 1. Create the Secret first

`fkst-control-plane-secret` is deliberately **not** in `kustomization.yaml` —
`apply -k` can never overwrite your real credentials with placeholders. Create
it first (the namespace must exist for the Secret to live in):

```sh
kubectl --context docker-desktop apply -f backend/fkst-control-plane/k8s_sample/namespace.yaml

TOK="$(openssl rand -hex 32)"   # share TOK with the worker Secret (#134)
kubectl --context docker-desktop -n fkst-hosted create secret generic fkst-control-plane-secret \
  --from-literal=FKST_INTERNAL_AUTH_TOKEN="$TOK"
```

**Consistency rule.** `FKST_INTERNAL_AUTH_TOKEN` MUST equal the worker Secret's
value (the controller↔worker shared bearer). The owner-only NyxID client (#257)
needs **no service-account credential**. There are **no MongoDB credentials** —
the controller is datastore-free (#143).

*Filled-file alternative:* copy `secret.example.yaml` to `secret.yaml`
(git-ignored — only the `.example` template stays tracked), replace every
`CHANGE_ME`, then `kubectl apply -f .../k8s_sample/secret.yaml`.

## 2. Deploy and verify

```sh
kubectl --context docker-desktop apply -k backend/fkst-control-plane/k8s_sample

kubectl --context docker-desktop -n fkst-hosted rollout status deployment/fkst-control-plane

kubectl --context docker-desktop -n fkst-hosted port-forward svc/fkst-hosted 8080:80
curl -s http://localhost:8080/health    # {"status":"ok","version":"..."}
curl -s http://localhost:8080/metrics   # fkst_pending_work / fkst_workers_* gauges
```

> **Images.** The `fkst-control-plane` image tag is a one-line seam in
> `kustomization.yaml` (`images:`). For real clusters, push to a registry and set
> `newName`/`newTag` there. On kind/minikube/k3d, load the image first
> (`kind load docker-image …` / `minikube image load …` / `k3d image import …`).

## 3. Datastore-free: no MongoDB

The controller holds **no datastore** (#143): no Mongo Service/StatefulSet, no DB
Secret, no `wait-for-mongodb` initContainer, no `MONGODB_*` keys.

- **Readiness == liveness.** `/health` probes nothing — a process that can answer
  the route is ready — so a startup is not gated on any external store. Liveness
  stays TCP-only (cheap, no dependency).
- **State on restart.** The controller's in-memory claim authority + worker
  registry + session store are rebuilt from the live workers' self-reports (they
  re-register + heartbeat on the controller's next boot) and the claimed
  goal-issue labels. The durable audit trail is the GitHub Issues themselves; the
  live ephemeral view is `GET /api/v1/admin/state` (admin-gated, secrets shown as
  presence booleans only) and `GET /metrics` (Prometheus, ClusterIP-only).

## 4. Transitional: engine execution (moves to the worker in #151)

Engine execution still lives in the control-plane crate today, so this
Deployment must run from the **engine-laden** image (point the
`fkst-control-plane` kustomize image at the `fkst-worker` image with a `command:`
override) and keeps its `runtime`/`tmp` emptyDir volumes. The
`FKST_HOSTED_ENGINE_*` ConfigMap keys apply to the control plane only until #151
moves engine execution onto the worker; then the controller switches to the
slim `fkst-control-plane` image, drops the `runtime` volume, and those keys move
to the worker's ConfigMap.

## 5. Single-replica operation (datastore-free)

The control-plane runs as a **single authoritative replica** (the in-memory claim
authority is single-writer). It is **not** an HA, multi-pod store anymore — the
elasticity lives entirely in the worker fleet.

- **At most one live session per lease key** is guaranteed by the controller's
  in-memory `ClaimMap` (single replica, single writer) — no cross-pod lease, no
  distributed CAS. A controller-issued monotonic `fencing_id` survives only as a
  journaling-idempotency / superseded-worker-rejection id, never for cross-pod
  arbitration.
- **Restart, not failover.** There is no survivor to take over. On a controller
  restart the in-memory state is **rebuilt** from the live workers' self-reports
  (they re-register + heartbeat) + the claimed goal-issue labels; in-flight goals
  are redone from GitHub truth (so redo is safe). A brief gap during a rollout is
  acceptable.
- **Recreate strategy.** `strategy.type: Recreate` (not `RollingUpdate`): a
  surging second pod would run a second, independent claim map — a split-brain —
  so the old pod is terminated before the new one starts.
  `terminationGracePeriodSeconds: 30` + `preStop: sleep 2` drain in-flight
  requests + SIGTERM live engines first.
- **PDB.** `maxUnavailable: 1` explicitly ALLOWS the single pod to be
  drained/recycled during voluntary disruptions (node drains, upgrades) — a
  `minAvailable: 2` would be unsatisfiable and would wedge every drain.
- **Pod identity.** `FKST_POD_ID` is wired explicitly from the downward API
  (`metadata.name`) so the controller's advisory `pod_id` (stamped on in-memory
  claims for journaling / load reflection) is the pod's stable, unique name.
- **Placement tuning.** `FKST_PLACEMENT_MAX_LOAD` (0 = uncapped) in
  `configmap.yaml` is the per-worker active-session cap consulted only under
  worker-dispatch mode. The lease/takeover cadence knobs are gone (datastore-free).
- **Observability.** `GET /api/v1/admin/state` (admin-gated) shows the live claim
  map, worker registry, and session store (secrets are presence-only); `GET
  /metrics` exposes `fkst_pending_work` / `fkst_workers_registered` /
  `fkst_workers_alive`. GitHub Issues remain the durable audit trail.

## 6. Teardown

```sh
kubectl --context docker-desktop delete namespace fkst-hosted   # removes every object in the namespace
```
