# fkst-worker on Kubernetes — SAMPLE manifests

Per-deployable **sample** Kubernetes objects for the **worker** fleet. Every
value here is an EXAMPLE — `backend/fkst-worker/src/config.rs` is the canonical
source of truth for the env contract. The control-plane's objects (and the
shared `fkst-hosted` Namespace) live in
[`backend/fkst-control-plane/k8s_sample/`](../../fkst-control-plane/k8s_sample/README.md).

## What this directory ships

| Object | Kind | Purpose |
|--------|------|---------|
| `fkst-worker` | Deployment (2 replicas) | Worker pods — **no public Service**; they connect UP to the controller and pull work |
| `fkst-worker-config` | ConfigMap | Non-secret runtime config (incl. `CONTROLLER_URL`, `FKST_WORKER_*`) |
| `fkst-worker-secret` | Secret | **Created by you, before apply** — `FKST_INTERNAL_AUTH_TOKEN` (template: `secret.example.yaml`) |
| `fkst-worker` | HorizontalPodAutoscaler | **Sample only** (#144/#140) — pending-work autoscaling shape |
| `fkst-worker` | PodDisruptionBudget | **Sample only** (#140) — worker drain floor |

`hpa.yaml` and `pdb.yaml` are clearly flagged samples for the topology that
**#144** (deployment topology) and **#140** (worker HPA + drain) finalize — not
the real wiring. `secret.example.yaml` is a **template only** (placeholder, not
in `kustomization.yaml`).

### No public Service

The worker exposes **no public Service and no Ingress**. It connects UP to the
controller over the stable control-plane Service DNS (`CONTROLLER_URL`) and
pulls work. It runs a worker-LOCAL HTTP server on `FKST_WORKER_PORT` (8090)
serving `GET /health` (`backend/fkst-worker/src/server.rs`) — used only by the
pod's startup/readiness probes, never fronted by a Service.

### Image-baked ENV (not set by any manifest)

`FKST_ROLE=worker`, `FKST_RUNTIME_ROOT=/var/lib/fkst/runtime` (read by the loader
as `engine_temp_root`), and `FKST_HOSTED_PORT` / `FKST_HOSTED_BIND_ADDR` are
baked into the worker image by the Dockerfile. `FKST_POD_ID` (= `worker_id`,
REQUIRED, fail-closed) is downward-API only (`fieldRef: metadata.name`).

## 1. Create the Secret first

`fkst-worker-secret` is deliberately **not** in `kustomization.yaml` — `apply -k`
can never overwrite your real token with a placeholder. The namespace must exist
first (it is declared by the control-plane dir):

```sh
kubectl --context docker-desktop apply -f backend/fkst-control-plane/k8s_sample/namespace.yaml

# The SAME token the control-plane Secret uses (#134):
TOK="$(openssl rand -hex 32)"
kubectl --context docker-desktop -n fkst-hosted create secret generic fkst-worker-secret \
  --from-literal=FKST_INTERNAL_AUTH_TOKEN="$TOK"
```

**Consistency rule.** `FKST_INTERNAL_AUTH_TOKEN` MUST equal the control plane's
`fkst-control-plane-secret` value, or the controller rejects the worker's
internal requests. Generate it once and use the same `$TOK` for both Secrets.

*Filled-file alternative:* copy `secret.example.yaml` to `secret.yaml`
(git-ignored), replace `CHANGE_ME`, then `kubectl apply -f .../k8s_sample/secret.yaml`.

## 2. Deploy and verify

The worker depends on a reachable controller. Bring up the control plane first
(see [`../../fkst-control-plane/k8s_sample/README.md`](../../fkst-control-plane/k8s_sample/README.md)),
then:

```sh
kubectl --context docker-desktop apply -k backend/fkst-worker/k8s_sample
kubectl --context docker-desktop -n fkst-hosted rollout status deployment/fkst-worker
kubectl --context docker-desktop -n fkst-hosted get pods -l app.kubernetes.io/name=fkst-worker
```

A worker becomes `Ready` as soon as its worker-local `/health` answers `200`.

> **Images.** The `fkst-worker` image tag is a one-line seam in
> `kustomization.yaml` (`images:`). The worker image is engine-laden; its engine
> pin (`FKST_SUBSTRATE_REF`) is **temporarily defaulted** to the
> `backend/engine.ref` SHA, so a no-arg `docker build` works (workaround, see
> #227 — pass `--build-arg FKST_SUBSTRATE_REF="$(cat backend/engine.ref)"` where
> the platform supports it). On kind/minikube/k3d, load the image first.

## 3. Controller handshake

The worker is **fail-closed**: `CONTROLLER_URL`, `FKST_INTERNAL_AUTH_TOKEN`, and
`FKST_POD_ID` are all REQUIRED — a missing/blank value stops the process at
startup. `CONTROLLER_URL` (`http://fkst-hosted:80`) is the stable in-cluster
Service DNS of the control-plane; `FKST_INTERNAL_AUTH_TOKEN` is the shared bearer
the worker presents on every internal request. The worker registers with the
controller's worker-liveness registry (TTL `FKST_WORKER_LIVENESS_TTL_SECS`, set
on the control plane) and pulls work every `FKST_WORKER_PULL_INTERVAL_SECS`.

## 4. Scaling

The **worker fleet is the elastic tier** — the control-plane is a single
datastore-free replica (#143/#144), so all horizontal scaling lives here. Workers
scale freely; the `replicas:` count in `kustomization.yaml` is the floor a fresh
apply starts from; once `#140` lands, the HorizontalPodAutoscaler governs the
live range on the pending-work metric (`fkst_pending_work`, exposed by the
controller's `/metrics`). Because the worker fleet self-registers and heartbeats
UP to the controller, a controller restart simply **rebuilds its in-memory state
from the live workers' self-reports** (and the claimed goal-issue labels) — the
fleet keeps running across a controller recycle. For an immediate change:

```sh
kubectl --context docker-desktop -n fkst-hosted scale deployment/fkst-worker --replicas=4
```

> An imperative `scale` is reverted by the next `apply -k`; for a durable change
> edit the `replicas:` count in `kustomization.yaml`.

`FKST_WORKER_DRAIN_GRACE_SECS` (25) bounds how long a draining worker spends
checkpointing + stopping in-flight sessions on SIGTERM; it MUST stay below the
Deployment's `terminationGracePeriodSeconds` (30) so the kubelet never SIGKILLs
mid-drain. The real drain/PDB policy is finalized by `#140`.
