# fkst-hosted on Kubernetes — local single-replica stack

Zero-to-running guide for the manifests in this directory. Primary target is
**Docker Desktop** (context `docker-desktop`); a portability box covers
kind/minikube/k3d and real registries.

## 1. What gets deployed

Everything lands in the **`fkst-hosted`** namespace:

| Resource | Kind | Purpose |
|----------|------|---------|
| `fkst-hosted-api` | Deployment (1 replica, `Recreate`) | The Rust API server, image `fkst-hosted:dev` |
| `fkst-hosted` | Service (ClusterIP, `80 → 8080`) | Cluster-internal entry to the API |
| `mongodb` | StatefulSet (1 replica) | Single-node MongoDB 7.0 with a retained PVC (`data-mongodb-0`, 5Gi) |
| `mongodb` | Headless Service | Stable DNS `mongodb-0.mongodb.fkst-hosted.svc.cluster.local` |
| `fkst-hosted-config` | ConfigMap | Non-secret runtime config |
| `fkst-hosted-secret` | Secret | **Created by you, before apply** (template: `secret.example.yaml`) |

Topology:

```
port-forward 8080 ──> svc/fkst-hosted:80 ──> api pod :8080 ──(MONGODB_URI)──> mongodb-0.mongodb:27017 ──> PVC data-mongodb-0
```

### Environment contract

| Variable | Source | Value | Notes |
|----------|--------|-------|-------|
| `MONGODB_DB` | ConfigMap | `fkst_hosted` | Logical database name |
| `MONGODB_SERVER_SELECTION_TIMEOUT_MS` | ConfigMap | `5000` | Bounds the startup ping and `/health` (coupled to probe timeouts, see §6) |
| `FKST_HOSTED_LOG_LEVEL` | ConfigMap | `info,fkst_hosted_api=debug,tower_http=info` | tracing-subscriber `EnvFilter` directive |
| `MONGODB_URI` | Secret | — | **Required, fail-closed**; embeds the root credentials, `authSource=admin` |
| `FKST_HOSTED_PORT` | image (`backend/Dockerfile` `ENV`) | `8080` | Baked into the image — not set by any manifest |
| `FKST_HOSTED_BIND_ADDR` | image | `0.0.0.0` | Baked into the image |
| `FKST_RUNTIME_ROOT` | image | `/var/lib/fkst/runtime` | Engine workspace; backed by the `runtime` emptyDir (see §6) |

## 2. Prerequisites

- **Docker Desktop** with Kubernetes enabled (Settings → Kubernetes → Enable),
  providing the `docker-desktop` kubectl context.
- **kubectl** (kustomize is built in via `kubectl apply -k`).

All commands run from the **repo root** and pin `--context docker-desktop` so a
stray current-context can never hit another cluster.

## 3. Build the image

The image bundles the API server **and** the fkst-substrate engine, compiled at
the ref pinned in `backend/engine.ref` (single-line commit SHA). The build
fails fast if `FKST_SUBSTRATE_REF` is not passed.

```sh
docker build -f backend/Dockerfile \
  --build-arg FKST_SUBSTRATE_REF="$(cat backend/engine.ref)" \
  -t fkst-hosted:dev .
```

Docker Desktop's Kubernetes shares the host Docker daemon, so the image is
immediately visible to the cluster — **no load/push step** (the Deployment uses
`imagePullPolicy: IfNotPresent`).

> **Other clusters.** Single-node dev clusters need an explicit image load:
>
> ```sh
> kind load docker-image fkst-hosted:dev        # kind
> minikube image load fkst-hosted:dev           # minikube
> k3d image import fkst-hosted:dev              # k3d
> ```
>
> For a real cluster, push to a registry and retarget the image in one place —
> the `images:` block in `kustomization.yaml`:
>
> ```sh
> docker tag fkst-hosted:dev registry.example.com/fkst-hosted:dev
> docker push registry.example.com/fkst-hosted:dev
> ```
>
> ```yaml
> # kustomization.yaml
> images:
>   - name: fkst-hosted
>     newName: registry.example.com/fkst-hosted
>     newTag: dev
> ```

## 4. Create the Secret (before apply)

`fkst-hosted-secret` is deliberately **not** in `kustomization.yaml` — `apply -k`
can never overwrite your real credentials with placeholders. Create it first
(the namespace must exist for the Secret to live in):

```sh
kubectl --context docker-desktop apply -f backend/deploy/k8s/namespace.yaml

PW="$(openssl rand -hex 16)"
kubectl --context docker-desktop -n fkst-hosted create secret generic fkst-hosted-secret \
  --from-literal=MONGO_INITDB_ROOT_USERNAME=root \
  --from-literal=MONGO_INITDB_ROOT_PASSWORD="$PW" \
  --from-literal=MONGODB_URI="mongodb://root:${PW}@mongodb-0.mongodb.fkst-hosted.svc.cluster.local:27017/fkst_hosted?authSource=admin"
```

**Consistency rule:** the `username:password` embedded in `MONGODB_URI` MUST
equal `MONGO_INITDB_ROOT_USERNAME` / `MONGO_INITDB_ROOT_PASSWORD`, and the URI
must keep `authSource=admin` (the root user lives in `admin`). A mismatch is
the single most common bring-up failure: Mongo comes up fine, the API stays
`NotReady` with auth errors.

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
kubectl --context docker-desktop -n fkst-hosted rollout status deployment/fkst-hosted-api
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
- **`Recreate` strategy is the single-instance contract.** The engine has no
  cross-host fencing, so a RollingUpdate's old+new pod overlap could run two
  engine instances for the same package. #27 (pool-manager) flips this to
  `RollingUpdate` once safe takeover exists.
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

## 8. Troubleshooting

| Symptom | Cause | Fix |
|---------|-------|-----|
| `ImagePullBackOff` / `ErrImageNeverPull` on the api pod | Image not built, or tag mismatch with the `images:` block in `kustomization.yaml` | Rebuild with `-t fkst-hosted:dev` (§3); on kind/minikube/k3d, run the load step |
| api pod `CrashLoopBackOff`, log says `MONGODB_URI must be set` (or another config error) | Secret missing, key misspelled, or created after the pod started | Create/fix `fkst-hosted-secret` (§4), then `kubectl --context docker-desktop -n fkst-hosted rollout restart deployment/fkst-hosted-api` |
| api pod stays `0/1 NotReady`; logs show Mongo auth errors | `MONGODB_URI` credentials drifted from `MONGO_INITDB_ROOT_*`, or the Secret was rotated after Mongo initialized | Restore consistency (§4); a true rotation requires the PVC wipe (§4 warning) |
| `port-forward` fails: `address already in use` | Local port 8080 is taken | Forward another local port, e.g. `kubectl --context docker-desktop -n fkst-hosted port-forward svc/fkst-hosted 18080:80` |
| api pod stuck in `Init:0/1` | Mongo not up yet (first image pull, PVC binding) — the initContainer is waiting for `mongodb-0:27017` | `kubectl --context docker-desktop -n fkst-hosted get pods,pvc` and wait, or inspect `describe pod mongodb-0` |
