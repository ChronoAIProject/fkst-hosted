# fkst-control-plane — Kubernetes sample

Sample manifests for the **single control-plane** deployable. The control plane
serves the public REST API and runs **pod-per-session**: it spawns one
Kubernetes Job per fkst-substrate session (the Job re-execs the SAME image in
`run-session` mode), watches it to completion, and GCs it. There is **no worker
deployable, no MongoDB, and no journaling** — those were removed.

`backend/src/config.rs` (+ `github_app/config.rs`) is the source of truth for the
env contract; every value here is a SAMPLE.

## Layout

| File | Purpose |
|------|---------|
| `rbac.yaml` | The control-plane SA, the session-runner SA, and the Role/RoleBinding that let the control plane create/watch/delete Jobs + per-session Secrets and read Pods. |
| `configmap.yaml` | Non-secret config (HTTP, NyxID proxy-trust auth, codex/chrono-llm, the `fkst` trigger label, `FKST_POD_*` dispatch). |
| `secret.example.yaml` | TEMPLATE for `fkst-control-plane-secret` (GitHub App creds + optional NyxID broker client). Excluded from kustomize. |
| `deployment.yaml` | The control plane (1 replica, Recreate). `FKST_POD_ID`/`FKST_POD_NAMESPACE` come from the downward API so session Jobs land in this namespace. |
| `service.yaml` | ClusterIP only (no Ingress). |
| `pdb.yaml` | `maxUnavailable: 1` (single authoritative replica). |
| `namespace.yaml` | OPTIONAL — only for a dedicated namespace; not part of the kustomization. |

## Deploy

```sh
# 1. Choose the namespace (kustomization.yaml `namespace:`) — it must already
#    exist (or apply namespace.yaml for a dedicated one).

# 2. Create the real Secret FIRST (never committed). The GitHub App enables the
#    webhook trigger + Job watcher; without it the API is up but nothing triggers.
kubectl -n <ns> create secret generic fkst-control-plane-secret \
  --from-literal=FKST_GITHUB_APP_ID="123456" \
  --from-literal=FKST_GITHUB_APP_WEBHOOK_SECRET="$(openssl rand -hex 32)" \
  --from-file=FKST_GITHUB_APP_PRIVATE_KEY_PEM=/path/to/app-key.pem
#    (Or create it empty to bring the control plane up App-less for now:
#     kubectl -n <ns> create secret generic fkst-control-plane-secret )

# 3. Build the image and apply. The image carries the control-plane binary +
#    engine + codex + nyxid; keep configmap FKST_POD_IMAGE in lockstep with the
#    kustomization image tag.
docker build -f backend/Dockerfile -t fkst-control-plane:dev .
kubectl apply -k backend/k8s_sample

# 4. Verify.
kubectl -n <ns> rollout status deploy/fkst-control-plane
kubectl -n <ns> port-forward svc/fkst-control-plane 8080:80 &
curl -s localhost:8080/health           # 200
curl -s localhost:8080/openapi.json     # live OpenAPI 3.1
```

## GitHub App webhook on a local cluster

The Service is ClusterIP-only, so GitHub can't reach it directly. Relay webhooks
with smee.io (set the App's webhook URL to the smee channel):

```sh
kubectl -n <ns> port-forward svc/fkst-control-plane 8080:80
npx smee-client -u https://smee.io/<channel> -t http://localhost:8080/api/v1/github/app/webhook
```

The App needs these repository permissions as **Read & write**: Administration,
Contents, Issues, Pull requests (Metadata read is implicit). Subscribe to BOTH
the **Issues** and the **Issue comment** events. A session triggers when an
installed repo gets an issue opened with the `fkst` label; once it exists, the
issue author drives it by commenting `/stop` or `/status` on the issue (the
**Issue comment** subscription is REQUIRED for those control commands to work).
