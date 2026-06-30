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

## Injecting per-user env into a session

The triggering issue body may carry an OPTIONAL `### Environment` section listing
env var **names** (one per line) to inject into the session — for example:

```md
### Goal
…

### Package Name List
…

### Environment
OPENAI_API_KEY
MY_FEATURE_FLAG
```

Each name is resolved against the **issue author's** own store, the
`fkst-user-<github_user_id>` ConfigMap (non-secret variables) + Secret (secret
values) in the control-plane namespace. Only the **named** keys are selected; a
name present in BOTH the variables and the secrets resolves to the secret value.
A requested name that the author has not stored is skipped (logged, not an
error), and if the store cannot be read the session still launches with no
injected env. Names must be valid env var names (`^[A-Za-z_][A-Za-z0-9_]*$`); a
malformed name fails issue parsing. Reserved/platform keys (anything `FKST_*`,
git-credential keys, or the engine's `LLM_API_KEY`) are dropped before reaching
the agent so a user value can never shadow a platform var.

The resolved values ride the per-session 0400 Secret as `userenv.<KEY>` files
and are folded into the agent's environment by the runner — they are NOT plain
pod env. A GitHub user populates their store via the
`/api/v1/users/me/env` and `/api/v1/users/me/secrets` API (authenticated by the
user's GitHub token; see PR4a). Secret values are write-only over that API — only
key names are ever returned.
