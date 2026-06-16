# fkst-control-plane on Kubernetes — SAMPLE manifests

Per-deployable **sample** Kubernetes objects for the **control-plane** (the
public REST API). Every value here is an EXAMPLE — the config loaders
(`backend/fkst-control-plane/src/config.rs` and its `leases/`, `distribution/`,
`reconcile/`, `github_app/` siblings, plus the linked
`backend/fkst-engine/src/config.rs`) are the canonical source of truth for the
env contract. The worker's objects live in
[`backend/fkst-worker/k8s_sample/`](../../fkst-worker/k8s_sample/README.md).

Primary target is **Docker Desktop** (context `docker-desktop`); the portability
notes cover kind/minikube/k3d and real registries.

## What this directory ships

Everything lands in the **`fkst-hosted`** Namespace (shared by both deployables;
declared here, not duplicated in the worker dir):

| Object | Kind | Purpose |
|--------|------|---------|
| `fkst-hosted` | Namespace | Shared namespace for both deployables |
| `fkst-control-plane` | Deployment (3 replicas, `RollingUpdate`) | The Rust control-plane (operating range 3–5, see "Multi-pod") |
| `fkst-control-plane` | PodDisruptionBudget (`minAvailable: 2`) | Keeps ≥2 control-plane pods available during voluntary disruptions |
| `fkst-hosted` | Service (ClusterIP, `80 → 8080`, **no Ingress**) | Cluster-internal entry to the control-plane (proxy-trust isolation) |
| `fkst-control-plane-config` | ConfigMap | Comprehensive non-secret runtime config |
| `fkst-control-plane-secret` | Secret | **Created by you, before apply** (template: `secret.example.yaml`) |
| `mongodb` | StatefulSet (1 replica) + headless Service | **Transitional** (removed by #143) — single-node MongoDB 7.0, retained PVC `data-mongodb-0` (5Gi) |

`secret.example.yaml` is a **template only** (placeholders, not in
`kustomization.yaml`) and is never a real Secret.

### No Ingress — proxy-trust isolation

The Service is **ClusterIP only**; there is **no Ingress**. fkst-hosted trusts
the NyxID-proxy-injected identity (issue #113) precisely because it is reachable
ONLY via the in-cluster NyxID proxy, never directly by a public client. Local
access is `kubectl -n fkst-hosted port-forward svc/fkst-hosted 8080:80`.

### Image-baked ENV (not set by any manifest)

`FKST_ROLE`, `FKST_HOSTED_PORT=8080`, `FKST_HOSTED_BIND_ADDR=0.0.0.0` are baked
into the image by the Dockerfile; `FKST_RUNTIME_ROOT=/var/lib/fkst/runtime` is
baked into the engine-laden image the control-plane transitionally runs from.
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

PW="$(openssl rand -hex 16)"
TOK="$(openssl rand -hex 32)"   # share TOK with the worker Secret (#134)
kubectl --context docker-desktop -n fkst-hosted create secret generic fkst-control-plane-secret \
  --from-literal=MONGO_INITDB_ROOT_USERNAME=root \
  --from-literal=MONGO_INITDB_ROOT_PASSWORD="$PW" \
  --from-literal=MONGODB_URI="mongodb://root:${PW}@mongodb-0.mongodb:27017/fkst_hosted?authSource=admin" \
  --from-literal=FKST_INTERNAL_AUTH_TOKEN="$TOK"
```

**Consistency rules.** The `username:password` in `MONGODB_URI` MUST equal
`MONGO_INITDB_ROOT_USERNAME` / `MONGO_INITDB_ROOT_PASSWORD` (the URI keeps
`authSource=admin`); a mismatch is the single most common bring-up failure
(control-plane stays `NotReady` with auth errors). `FKST_INTERNAL_AUTH_TOKEN`
MUST equal the worker Secret's value. The same Secret is read by the Mongo
StatefulSet (for `MONGO_INITDB_ROOT_*`) and by the control-plane container.

*Filled-file alternative:* copy `secret.example.yaml` to `secret.yaml`
(git-ignored — only the `.example` template stays tracked), replace every
`CHANGE_ME`, then `kubectl apply -f .../k8s_sample/secret.yaml`.

> **Credential rotation.** `MONGO_INITDB_ROOT_*` initializes the root user only
> on **first boot against an empty data dir**. Changing the Secret after Mongo
> has initialized does nothing until you wipe the PVC (`delete pvc data-mongodb-0`
> with the StatefulSet scaled to 0 — this **deletes all data**).

## 2. Deploy and verify

```sh
kubectl --context docker-desktop apply -k backend/fkst-control-plane/k8s_sample

kubectl --context docker-desktop -n fkst-hosted rollout status statefulset/mongodb
kubectl --context docker-desktop -n fkst-hosted rollout status deployment/fkst-control-plane

kubectl --context docker-desktop -n fkst-hosted port-forward svc/fkst-hosted 8080:80
curl -s http://localhost:8080/health   # {"status":"ok","mongo":"up","version":"..."}
```

> **Images.** The `fkst-control-plane` image tag is a one-line seam in
> `kustomization.yaml` (`images:`). For real clusters, push to a registry and set
> `newName`/`newTag` there. On kind/minikube/k3d, load the image first
> (`kind load docker-image …` / `minikube image load …` / `k3d image import …`).

## 3. Transitional: MongoDB (removed by #143)

The Mongo Service + StatefulSet and the control-plane Deployment's
`wait-for-mongodb` initContainer exist **only while the control plane is
DB-backed**. They are clearly flagged "transitional until #143" in each file's
header. When #143 lands the DB-free pivot, this whole Mongo pair and all
`MONGODB_*` / `MONGO_INITDB_ROOT_*` keys are deleted mechanically.

- **Fail-closed startup.** The server pings Mongo at startup and exits if the
  store is unreachable; the busybox initContainer waits for `mongodb-0…:27017`
  to accept TCP, absorbing first-boot ordering without weakening the fail-closed
  contract.
- **Readiness gates on Mongo; liveness does not.** `/health` answers `503`
  when Mongo is unreachable, so the readiness probe marks the pod `NotReady`
  **without restarts**. Liveness is TCP-only by design — a DB blip must not
  restart-loop the API. Probe `timeoutSeconds: 7` is coupled to
  `MONGODB_SERVER_SELECTION_TIMEOUT_MS=5000`; raise both together.

## 4. Transitional: engine execution (moves to the worker in #151)

Engine execution still lives in the control-plane crate today, so this
Deployment must run from the **engine-laden** image (point the
`fkst-control-plane` kustomize image at the `fkst-worker` image with a `command:`
override) and keeps its `runtime`/`tmp` emptyDir volumes. The
`FKST_HOSTED_ENGINE_*` ConfigMap keys apply to the control plane only until #151
moves engine execution onto the worker; then the controller switches to the
slim `fkst-control-plane` image, drops the `runtime` volume, and those keys move
to the worker's ConfigMap.

## 5. Multi-pod operation (3–5 replicas)

The control-plane runs as **3 replicas** by default (operating range **3–5**)
sharing the single MongoDB lease store.

- **At most one live session per package** is guaranteed by a **per-package
  lease** in Mongo, not by the deployment strategy. Every engine spawn / session
  write is tagged with the lease holder's **fencing token**, so a stale
  (taken-over) holder is fenced out.
- **Failover is lease-driven.** A pod that loses its lease self-fences; an
  expired lease (hard pod loss) is taken over by a survivor's reaper, which
  redoes the session from scratch (GitHub is the source of truth, so redo is
  safe).
- **Scaling.** The replica count is the `replicas:` edit-point in
  `kustomization.yaml`. Keep it within **3–5** so the PDB (`minAvailable: 2`)
  always has headroom.
- **Rolling updates.** `maxUnavailable: 0` / `maxSurge: 1` keep full capacity
  during a rollout; `terminationGracePeriodSeconds: 30` + `preStop: sleep 2`
  drain the pod and release its leases before SIGKILL.
- **Pod identity.** `FKST_POD_ID` is wired explicitly from the downward API
  (`metadata.name`) so the distribution layer's `holder_pod` / `pod_id` is the
  pod's stable, unique name.
- **Lease / takeover tuning.** All knobs live in `configmap.yaml` at their
  in-code defaults: `FKST_LEASE_TTL_SECS` (30), `FKST_LEASE_RENEW_INTERVAL_SECS`
  (10), `FKST_TAKEOVER_SCAN_INTERVAL_SECS` (5), `FKST_TAKEOVER_GRACE_SECS` (2),
  `FKST_PLACEMENT_MAX_LOAD` (0 = uncapped). The loader is fail-closed:
  `TTL ∈ 1..=86400`; `0 < RENEW` and `RENEW*2 < TTL`; `SCAN > 0`; `GRACE ≥ 0`.
  These keys are likely removed by #143/#144 (DB-free pivot).
- **High availability.** The PDB guards voluntary disruptions (node drains,
  rollouts, upgrades) only; involuntary loss is covered by lease expiry + reaper
  takeover. Soft pod anti-affinity spreads replicas across nodes when available
  (preferred, not required — single-node docker-desktop still schedules all 3).

## 6. Teardown

```sh
kubectl --context docker-desktop delete namespace fkst-hosted   # deletes the PVC too — full data wipe
```
