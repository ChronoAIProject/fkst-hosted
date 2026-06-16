# fkst-hosted on Kubernetes — local multi-pod stack

Zero-to-running guide for the manifests in this directory. Primary target is
**Docker Desktop** (context `docker-desktop`); a portability box covers
kind/minikube/k3d and real registries.

The control-plane runs as **3 replicas** (operating range 3–5) behind one
MongoDB lease store. Multi-pod operation, scaling, rolling updates, failover,
and the lease/takeover tuning knobs are documented in §9 — read it before
scaling or operating more than one replica.

## 1. What gets deployed

Everything lands in the **`fkst-hosted`** namespace:

| Resource | Kind | Purpose |
|----------|------|---------|
| `fkst-control-plane` | Deployment (3 replicas, `RollingUpdate`) | The Rust control-plane, image `fkst-control-plane:dev` (operating range 3–5, see §9) |
| `fkst-control-plane` | PodDisruptionBudget (`minAvailable: 2`) | Keeps ≥2 control-plane pods available during voluntary disruptions (see §9.6) |
| `fkst-hosted` | Service (ClusterIP, `80 → 8080`) | Cluster-internal entry to the control-plane |
| `fkst-worker` | Deployment | Worker pods; no public Service — workers connect up to the controller and pull work |
| `mongodb` | StatefulSet (1 replica) | Single-node MongoDB 7.0 with a retained PVC (`data-mongodb-0`, 5Gi) |
| `mongodb` | Headless Service | Stable DNS `mongodb-0.mongodb.fkst-hosted.svc.cluster.local` |
| `fkst-control-plane-config` | ConfigMap | Non-secret runtime config for the control-plane |
| `fkst-worker-config` | ConfigMap | Non-secret runtime config for workers (log level for now) |
| `fkst-control-plane-secret` | Secret | **Created by you, before apply** — MongoDB URI, NyxID credentials, GitHub App PEM (template: `secret.example.yaml`) |

Topology:

```
port-forward 8080 ──> svc/fkst-hosted:80 ──> control-plane pod :8080 ──(MONGODB_URI)──> mongodb-0.mongodb:27017 ──> PVC data-mongodb-0
                                                                                 ↑
                                                           worker pods (connect up, no inbound Service)
```

### Environment contract

| Variable | Source | Value | Notes |
|----------|--------|-------|-------|
| `MONGODB_DB` | ConfigMap (`fkst-control-plane-config`) | `fkst_hosted` | Logical database name |
| `MONGODB_SERVER_SELECTION_TIMEOUT_MS` | ConfigMap (`fkst-control-plane-config`) | `5000` | Bounds the startup ping and `/health` (coupled to probe timeouts, see §6) |
| `FKST_HOSTED_LOG_LEVEL` | ConfigMap (`fkst-control-plane-config`) | `info,fkst_control_plane=debug,tower_http=info` | tracing-subscriber `EnvFilter` directive |
| `MONGODB_URI` | Secret (`fkst-control-plane-secret`) | — | **Required, fail-closed**; embeds the root credentials, `authSource=admin` |
| `FKST_HOSTED_PORT` | image (`backend/Dockerfile` `ENV`) | `8080` | Baked into the image — not set by any manifest |
| `FKST_HOSTED_BIND_ADDR` | image | `0.0.0.0` | Baked into the image |
| `FKST_RUNTIME_ROOT` | image | `/var/lib/fkst/runtime` | Engine workspace; backed by the `runtime` emptyDir (see §6) |

## 2. Prerequisites

- **Docker Desktop** with Kubernetes enabled (Settings → Kubernetes → Enable),
  providing the `docker-desktop` kubectl context.
- **kubectl** (kustomize is built in via `kubectl apply -k`).

All commands run from the **repo root** and pin `--context docker-desktop` so a
stray current-context can never hit another cluster.

## 3. Build the images

`backend/Dockerfile` produces **two images** via `--target`. Both are referenced
by `kustomization.yaml` under `images:` (`fkst-control-plane` and `fkst-worker`).

```sh
# Control-plane image
# TRANSITIONAL: build from --target worker-runtime (engine-laden) until issue #136
# moves engine execution onto the worker; then switch to --target control-plane-runtime (slim).
docker build -f backend/Dockerfile --target worker-runtime \
  --build-arg FKST_SUBSTRATE_REF="$(cat backend/engine.ref)" \
  -t fkst-control-plane:dev .

# Worker image (engine-laden; needs FKST_SUBSTRATE_REF)
docker build -f backend/Dockerfile --target worker-runtime \
  --build-arg FKST_SUBSTRATE_REF="$(cat backend/engine.ref)" \
  -t fkst-worker:dev .
```

> **Post-#136 (slim control-plane).** Once issue #136 lands and engine execution
> moves to the worker, rebuild the control-plane from the slim target instead:
>
> ```sh
> docker build -f backend/Dockerfile --target control-plane-runtime \
>   -t fkst-control-plane:dev .
> ```
>
> At that point the control-plane Deployment also drops its `runtime` emptyDir
> volume and the `FKST_SUBSTRATE_REF` arg is no longer needed for it.

Docker Desktop's Kubernetes shares the host Docker daemon, so built images are
immediately visible to the cluster — **no load/push step** (Deployments use
`imagePullPolicy: IfNotPresent`).

> **Other clusters.** Single-node dev clusters need an explicit image load:
>
> ```sh
> kind load docker-image fkst-control-plane:dev   # kind
> kind load docker-image fkst-worker:dev
> minikube image load fkst-control-plane:dev       # minikube
> minikube image load fkst-worker:dev
> k3d image import fkst-control-plane:dev          # k3d
> k3d image import fkst-worker:dev
> ```
>
> For a real cluster, push to a registry and retarget both images in the
> `images:` block of `kustomization.yaml`:
>
> ```yaml
> # kustomization.yaml
> images:
>   - name: fkst-control-plane
>     newName: registry.example.com/fkst-control-plane
>     newTag: dev
>   - name: fkst-worker
>     newName: registry.example.com/fkst-worker
>     newTag: dev
> ```

## 4. Create the Secret (before apply)

`fkst-control-plane-secret` is deliberately **not** in `kustomization.yaml` —
`apply -k` can never overwrite your real credentials with placeholders. Create
it first (the namespace must exist for the Secret to live in):

```sh
kubectl --context docker-desktop apply -f backend/deploy/k8s/namespace.yaml

PW="$(openssl rand -hex 16)"
kubectl --context docker-desktop -n fkst-hosted create secret generic fkst-control-plane-secret \
  --from-literal=MONGO_INITDB_ROOT_USERNAME=root \
  --from-literal=MONGO_INITDB_ROOT_PASSWORD="$PW" \
  --from-literal=MONGODB_URI="mongodb://root:${PW}@mongodb-0.mongodb:27017/fkst_hosted?authSource=admin"
```

**Consistency rule:** the `username:password` embedded in `MONGODB_URI` MUST
equal `MONGO_INITDB_ROOT_USERNAME` / `MONGO_INITDB_ROOT_PASSWORD`, and the URI
must keep `authSource=admin` (the root user lives in `admin`). A mismatch is
the single most common bring-up failure: Mongo comes up fine, the control-plane
stays `NotReady` with auth errors. (Single-secret design: the control-plane
container also receives `MONGO_INITDB_ROOT_USERNAME`/`MONGO_INITDB_ROOT_PASSWORD`
via the shared Secret's `envFrom` — harmless extras the control-plane does not
read.)

*Filled-file alternative:* copy `secret.example.yaml` to
`backend/deploy/k8s/secret.yaml` (git-ignored by the root `.gitignore`, as is
`secret.*.local.yaml` — only the `.example` template stays tracked), replace
every `CHANGE_ME`, then `kubectl --context docker-desktop apply -f backend/deploy/k8s/secret.yaml`.

> **Warning — credential rotation.** `MONGO_INITDB_ROOT_*` initializes the root
> user only on **first boot against an empty data dir**. Changing the Secret
> after Mongo has initialized does nothing until you wipe the PVC
> (`kubectl --context docker-desktop -n fkst-hosted delete pvc data-mongodb-0`
> with the StatefulSet scaled to 0 — this **deletes all data**).

## 5. Deploy and verify

```sh
kubectl --context docker-desktop apply -k backend/deploy/k8s

kubectl --context docker-desktop -n fkst-hosted rollout status statefulset/mongodb
kubectl --context docker-desktop -n fkst-hosted rollout status deployment/fkst-control-plane
kubectl --context docker-desktop -n fkst-hosted rollout status deployment/fkst-worker
```

(`namespace.yaml` is also in the kustomization, so re-applying it is harmless.)

Port-forward and hit the health endpoint:

```sh
kubectl --context docker-desktop -n fkst-hosted port-forward svc/fkst-hosted 8080:80
```

```sh
curl -s http://localhost:8080/health
```

Expected `200` body:

```json
{"status":"ok","mongo":"up","version":"0.0.0"}
```

Package API smoke (a package must contain an engine entry file —
`departments/<name>/main.lua` or `raisers/<name>.lua`):

```sh
curl -si -X POST http://localhost:8080/api/v1/packages \
  -H 'content-type: application/json' \
  -d '{"name":"demo","files":[{"path":"departments/demo/main.lua","content":"-- demo entry\n"}]}'
# HTTP/1.1 201 Created
# location: /api/v1/packages/demo
# {"name":"demo"}

curl -s http://localhost:8080/api/v1/packages
# ["demo"]

curl -s http://localhost:8080/api/v1/packages/demo
# {"name":"demo","files":[...],"composed_deps":[],"created_at":"...Z","updated_at":"...Z"}
```

## 6. Behavior notes (read before "fixing" anything)

- **Fail-closed startup + `wait-for-mongodb` initContainer.** The server pings
  Mongo at startup and exits if the store is unreachable. The busybox
  initContainer waits for `mongodb-0...:27017` to accept TCP, absorbing
  first-boot ordering without weakening the fail-closed contract.
- **Readiness gates on Mongo; liveness does not.** `/health` answers `503`
  `{"status":"degraded","mongo":"down",...}` when Mongo is unreachable, so the
  readiness probe marks the pod `NotReady` (removed from the Service) **without
  restarts**. The liveness probe is TCP-only by design — if it depended on
  Mongo, a DB blip would restart-loop a healthy API.
- **Probe `timeoutSeconds: 7` is coupled to `MONGODB_SERVER_SELECTION_TIMEOUT_MS=5000`.**
  `/health`'s Mongo ping is bounded at 5s, so the HTTP probes allow 7s for the
  degraded `503` to arrive instead of timing out. If you raise the ConfigMap
  timeout, raise the startup/readiness probe timeouts to stay above it.
- **`enableServiceLinks: false` is REQUIRED.** The Service is named
  `fkst-hosted`, so Kubernetes' Docker-link-style injection would set
  `FKST_HOSTED_PORT=tcp://<clusterIP>:80` in the pod — colliding with the API's
  `FKST_HOSTED_*` env prefix, where it parses as `PORT` (a `u16`) and fails the
  fail-closed config loader at startup. This was found live; do not "clean it
  up". The API's env comes exclusively from the image, ConfigMap, and Secret.
- **`RollingUpdate` is safe because the lease, not the strategy, serializes
  execution.** At most one pod holds a given package's lease at a time, and
  every engine spawn / session write is tagged with the holder's fencing token,
  so a stale (taken-over) holder is fenced out. `maxUnavailable: 0` /
  `maxSurge: 1` keep full capacity during a rollout. See §9 for the full
  multi-pod model, scaling, and failover.
- **Engine runtime is ephemeral.** The root FS is read-only; the `runtime`
  emptyDir at `/var/lib/fkst/runtime` (= the image's `FKST_RUNTIME_ROOT`, the
  #16 image contract) is the engine workspace — local, intentionally ephemeral
  (redo-from-GitHub-truth model), never a PVC.

## 7. Operate

**Degraded-path demo** — readiness flips, no restarts:

```sh
kubectl --context docker-desktop -n fkst-hosted scale statefulset/mongodb --replicas=0
curl -s http://localhost:8080/health   # 503 {"status":"degraded","mongo":"down",...}
kubectl --context docker-desktop -n fkst-hosted get pods   # api pod 0/1 NotReady, RESTARTS unchanged

kubectl --context docker-desktop -n fkst-hosted scale statefulset/mongodb --replicas=1
# api pod returns to 1/1 Ready on the next readiness pass, without restarting
```

**PVC persistence** — data survives pod deletion:

```sh
kubectl --context docker-desktop -n fkst-hosted delete pod mongodb-0
# the StatefulSet recreates mongodb-0 on the same PVC (data-mongodb-0)
curl -s http://localhost:8080/api/v1/packages   # ["demo"] — still there
```

**Teardown** — deletes everything **including the PVC** (full data wipe):

```sh
kubectl --context docker-desktop delete namespace fkst-hosted
```

## 9. Multi-pod operation (3–5 replicas)

This stack runs the API as **3 replicas** by default (operating range **3–5**)
sharing the single MongoDB lease store. This section is the operations runbook
for that posture.

### 9.1 Overview — how multi-pod stays correct

- All replicas share one MongoDB store. The Service (`fkst-hosted`)
  load-balances API calls across the ready pods.
- **At most one live session per package** is guaranteed by a **per-package
  lease** in Mongo, not by the deployment strategy. A `POST` that starts a
  session acquires the package's lease; a second `POST` for a package that
  already has a live session is rejected with **`409 Conflict`**.
- Every engine spawn / session write is tagged with the lease holder's
  **fencing token**, so a stale (taken-over) holder is fenced out — two engines
  never run the same package concurrently.
- **Failover/redo is lease-driven:**
  - A pod that **loses its lease self-fences** (stops its now-stale engine work
    rather than racing the new holder).
  - An **expired** lease (hard pod loss) is **taken over by a survivor's
    reaper**, which **redoes the session from scratch**. Engine runtime state is
    local and ephemeral — **GitHub is the source of truth**, so redo is safe.

```mermaid
graph TD
    SVC[Service fkst-hosted<br/>load-balances API calls] --> P1[pod A]
    SVC --> P2[pod B]
    SVC --> P3[pod C]
    P1 -.per-package lease + fencing token.-> M[(MongoDB<br/>lease store)]
    P2 -.-> M
    P3 -.-> M
    M -->|expired lease| R[survivor reaper<br/>takeover + redo from GitHub]
```

### 9.2 Scaling within the range

The replica count is a **one-line edit-point** in `kustomization.yaml` (the
`replicas:` block — kustomize overrides the Deployment's `spec.replicas`). Keep
it within **3–5** so the PodDisruptionBudget (`minAvailable: 2`) always has
headroom.

```yaml
# kustomization.yaml — change 3 ↔ 5 here, then re-apply
replicas:
  - name: fkst-control-plane
    count: 5
```

```sh
kubectl --context docker-desktop apply -k backend/deploy/k8s
kubectl --context docker-desktop -n fkst-hosted rollout status deployment/fkst-control-plane
```

For an immediate, imperative change (e.g. a quick scale-out during an incident):

```sh
kubectl --context docker-desktop -n fkst-hosted scale deployment/fkst-control-plane --replicas=5
kubectl --context docker-desktop -n fkst-hosted rollout status deployment/fkst-control-plane
```

> An imperative `scale` is reverted by the next `apply -k`. For a durable change,
> edit the `replicas:` count in `kustomization.yaml`.

### 9.3 Rolling updates (zero capacity gap)

```sh
# Restart all pods (e.g. to pick up a ConfigMap/Secret change)
kubectl --context docker-desktop -n fkst-hosted rollout restart deployment/fkst-control-plane

# Ship a new image tag
kubectl --context docker-desktop -n fkst-hosted set image \
  deployment/fkst-control-plane fkst-control-plane=fkst-control-plane:dev

kubectl --context docker-desktop -n fkst-hosted rollout status deployment/fkst-control-plane
```

- **`maxUnavailable: 0` / `maxSurge: 1`** means the rollout surges one extra pod
  before removing an old one — **no capacity gap**. Readiness (`/health`,
  Mongo-gated) keeps a pod out of the Service until it is serving, so the
  Service always points at ready pods only. The verified drill observed **0
  non-200 `/health` responses during a rollout**.
- **`terminationGracePeriodSeconds: 30` + `preStop: sleep 2`** give a draining
  pod time to deregister from the Service (the `sleep 2` lets the endpoints
  controller drop it while readiness is still passing) and then **gracefully
  stop its sessions — releasing their leases** — before `SIGKILL`. A survivor
  picks up released leases without waiting for TTL expiry.

### 9.4 Pod identity

Each replica needs a **stable, unique lease-holder id**. `FKST_POD_ID` is
sourced explicitly from the **downward API** (`fieldRef: metadata.name`) in the
Deployment, so the distribution layer's `holder_pod` / `pod_id` is the pod's
unique name.

Resolution precedence in the config loader:

```
FKST_POD_ID  →  HOSTNAME  →  local-<uuid>
```

Wiring `FKST_POD_ID` explicitly from `metadata.name` makes identity
**intentional** (not the ambient `HOSTNAME` coincidence) and defense-in-depth so
it never silently degrades to a random `local-<uuid>`.

### 9.5 Lease / takeover tuning

All knobs live in `configmap.yaml` and are set to their in-code defaults, made
explicit so the multi-pod coordination posture is visible and tunable without
rebuilding the image.

| Env var | Default | Meaning |
|---------|---------|---------|
| `FKST_LEASE_TTL_SECS` | `30` | Lease lifetime. A lease not renewed within the TTL is considered expired and eligible for takeover. |
| `FKST_LEASE_RENEW_INTERVAL_SECS` | `10` | How often the holder renews its lease. Must satisfy `RENEW*2 < TTL`. |
| `FKST_TAKEOVER_SCAN_INTERVAL_SECS` | `5` | How often a survivor's reaper scans for expired leases to take over. |
| `FKST_TAKEOVER_GRACE_SECS` | `2` | Extra grace beyond TTL before a survivor claims an expired lease. `0` is allowed. |
| `FKST_PLACEMENT_MAX_LOAD` | `0` | Per-pod active-session cap for placement. `0` = uncapped. |

- **Invariant: `RENEW*2 < TTL`** (here `10*2 = 20 < 30`) — two renew attempts
  must fit inside the TTL so a single missed renewal never expires a healthy
  lease. The config loader is **fail-closed**: it rejects values that break the
  constraints (`TTL` in `1..=86400`; `0 < RENEW` and `RENEW*2 < TTL`; `SCAN > 0`;
  `GRACE ≥ 0`; `MAX_LOAD ≥ 0`).
- **Failover timing:**
  - **Graceful release** (rollout / scale-in / clean SIGTERM): the lease is
    released immediately; a survivor reclaims it within **≤ one scan interval**
    (`SCAN`, default 5s).
  - **Hard pod loss** (kill / node death — no release): takeover happens within
    **≤ TTL + grace** (default `30 + 2 = 32s`) once the lease expires and the
    next reaper scan picks it up.

### 9.6 High availability

- **PodDisruptionBudget (`minAvailable: 2`).** During **voluntary** disruptions
  (node drains, `kubectl rollout`, cluster upgrades) Kubernetes keeps ≥2 pods
  available, so the API never drops below 2-pod capacity while one is recycled.
  It does **not** guard against involuntary loss (hard pod/node death) — that
  path is covered by lease expiry + reaper takeover (§9.5).
- **Preferred (soft) pod anti-affinity** spreads replicas across nodes
  (`topologyKey: kubernetes.io/hostname`) **when nodes are available**. It is
  intentionally *preferred*, not *required*: single-node **docker-desktop** still
  schedules all 3 replicas (a required rule would leave replicas 2..N `Pending`
  forever). On a real multi-node cluster this nudges spreading for fault
  tolerance without ever blocking scheduling.

### 9.7 Known limitation — placement uses `SelfOnlyHealth` (v1)

In v1, placement uses **`SelfOnlyHealth`**: each pod places the work it receives
**on itself**. There is no cross-pod, load-aware placement yet.

This is a **load-distribution deferral, not a correctness gap**:

- The Service load-balances API calls across pods, and the **per-package lease
  arbitrates** execution — so **no two pods ever run the same package
  concurrently**, regardless of where work is placed.
- Cross-pod, load-aware placement (honoring `FKST_PLACEMENT_MAX_LOAD` across the
  whole fleet) is deferred behind the **`HealthView` seam** — a future
  `RegistryHealth` / heartbeat source of truth — and can land without changing
  the lease/fencing correctness model.

### 9.8 Failover verification (operator drill)

```sh
# 1. Bring up the stack at 3 replicas; confirm all Ready
kubectl --context docker-desktop apply -k backend/deploy/k8s
kubectl --context docker-desktop -n fkst-hosted rollout status deployment/fkst-control-plane
kubectl --context docker-desktop -n fkst-hosted get pods -l app.kubernetes.io/name=fkst-control-plane

# 2. Start one or more sessions (use a LONG-RUNNING package — see note below)

# 3. Identify the lease holder for a live session, then kill that pod
kubectl --context docker-desktop -n fkst-hosted delete pod <holder-pod>

# 4. Observe a survivor take over within ~TTL+grace (~32s):
#    the session's pod_id changes to a survivor and its fencing_token increments
kubectl --context docker-desktop -n fkst-hosted logs -l app.kubernetes.io/name=fkst-control-plane -f
```

- **Use a long-lived package for a live demo.** The bundled `e2e-minimal`
  completes too fast to observe a mid-flight takeover — by the time you delete
  the holder, the session is already terminal. Use a longer-running package so
  the session is still active when the pod dies.
- **The takeover/redo logic is covered deterministically** by the distribution
  integration tests (`tests/distribution.rs`) — the operator drill is a live
  confidence check, not the primary correctness guarantee.

## 10. Troubleshooting

| Symptom | Cause | Fix |
|---------|-------|-----|
| `ImagePullBackOff` / `ErrImageNeverPull` on a pod | Image not built, or tag mismatch with the `images:` block in `kustomization.yaml` | Rebuild with `-t fkst-control-plane:dev` or `-t fkst-worker:dev` (§3); on kind/minikube/k3d, run the load step |
| control-plane pod `CrashLoopBackOff`, log says `MONGODB_URI must be set` (or another config error) | Secret missing, key misspelled, or created after the pod started | Create/fix `fkst-control-plane-secret` (§4), then `kubectl --context docker-desktop -n fkst-hosted rollout restart deployment/fkst-control-plane` |
| control-plane pod stays `0/1 NotReady`; logs show Mongo auth errors | `MONGODB_URI` credentials drifted from `MONGO_INITDB_ROOT_*`, or the Secret was rotated after Mongo initialized | Restore consistency (§4); a true rotation requires the PVC wipe (§4 warning) |
| `port-forward` fails: `address already in use` | Local port 8080 is taken | Forward another local port, e.g. `kubectl --context docker-desktop -n fkst-hosted port-forward svc/fkst-hosted 18080:80` |
| control-plane pod stuck in `Init:0/1` | Mongo not up yet (first image pull, PVC binding) — the initContainer is waiting for `mongodb-0:27017` | `kubectl --context docker-desktop -n fkst-hosted get pods,pvc` and wait, or inspect `describe pod mongodb-0` |
