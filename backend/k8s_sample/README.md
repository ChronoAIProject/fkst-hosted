# fkst-control-plane — Kubernetes sample

Sample manifests for the **single control-plane** deployable. The control plane
serves the public REST API and runs **pod-per-session**: it spawns one
Kubernetes Job per fkst-substrate session (the Job re-execs the SAME image in
`run-session` mode), watches it to completion, and GCs it. There is **no worker
deployable, no MongoDB, and no journaling** — those were removed.

`backend/src/config.rs` (+ `github_app/config.rs`) is the source of truth for the
env contract; every value here is a SAMPLE.

## Execution modes

The control plane dispatches each session through one of two backends, selected
by `FKST_POD_MODE` (`configmap.yaml`). Everything else in this README describes the
**default**, `k8s-customized`; the manifests deploy **either** mode. The only
per-mode differences are which RBAC file you apply and which env families you set.

### Choosing an RBAC posture

| Mode | `FKST_POD_MODE` | RBAC file to apply | Required env families | Who owns pod authority |
|------|-----------------|--------------------|-----------------------|------------------------|
| **k8s-customized** (default) | `k8s-customized` (or unset) | `rbac.yaml` | `FKST_POD_*` (image, namespace, service account, TTLs, runtime class) | **This repo's RBAC** — the control plane creates/watches/GCs the session Pods itself. |
| **opensandbox** | `opensandbox` | `rbac-opensandbox.yaml` | `FKST_POD_MODE=opensandbox` + `FKST_OSB_*` (base URL, CPU/mem, entrypoint, proxy) + still `FKST_POD_IMAGE` and `FKST_POD_NAMESPACE` | **The OpenSandbox server infra** — its own ServiceAccount runs the sandboxes; the control plane gets ZERO pod RBAC. |

Both RBAC files grant the control plane the SAME `secrets` + `configmaps` slice
for the per-user env_store (`backend/src/k8s/env_store.rs`, the `fkst-env-<id>-<name>`
ConfigMap + Secret pairs) — control-plane state storage, needed in both modes. They
differ ONLY in the pod surface: `rbac.yaml` adds the full `pods`/`pods/log`/`pods/status`
launcher rules; `rbac-opensandbox.yaml` adds none. **Apply exactly one**, matching
`FKST_POD_MODE`. Before enabling opensandbox mode in a real cluster, walk the
[Infra prerequisites (opensandbox mode)](#infra-prerequisites-opensandbox-mode) and
the [Security-review gate (opensandbox mode)](#security-review-gate-opensandbox-mode).

## Layout

| File | Purpose |
|------|---------|
| `rbac.yaml` | **k8s-customized-mode** RBAC: the control-plane SA (create/watch/delete Pods, Secrets, ConfigMaps + the env-validation Pods, read Pods + pod logs) and the DELIBERATELY zero-RBAC `fkst-session-runner` SA the session/validation pods run as. |
| `rbac-opensandbox.yaml` | **opensandbox-mode** RBAC: the same control-plane SA with the env_store `secrets`/`configmaps` slice ONLY and ZERO pod RBAC (the OpenSandbox server owns the session runtime). Apply this INSTEAD of `rbac.yaml` when `FKST_POD_MODE=opensandbox` — see "Choosing an RBAC posture". |
| `configmap.yaml` | Non-secret config (HTTP, codex/chrono-llm — the LLM base URL is the PUBLIC host so isolated pods reach it externally, the trigger label, `FKST_POD_*`/`FKST_ENV_*` dispatch). |
| `networkpolicy.yaml` | Session/validation-pod hard isolation (#338 R3): deny all ingress, egress to the public internet only. No-op unless the CNI enforces NetworkPolicy — set the real pod/service CIDRs first. |
| `secret.example.yaml` | TEMPLATE for `fkst-control-plane-secret` (GitHub App creds). Excluded from kustomize. |
| `deployment.yaml` | The control plane (1 replica, Recreate). `FKST_POD_ID`/`FKST_POD_NAMESPACE` come from the downward API so session Jobs land in this namespace. |
| `service.yaml` | ClusterIP only (no Ingress). |
| `pdb.yaml` | `maxUnavailable: 1` (single authoritative replica). |
| `namespace.yaml` | OPTIONAL — only for a dedicated namespace; carries the `pod-security.kubernetes.io/enforce: baseline` label (apply it to your namespace if you reuse an existing one). |

### Session-pod hard isolation (#338 R3)

A session pod (and the ephemeral env-validation pod) runs **untrusted agent code + arbitrary root install commands**, so it must never reach anything else in the cluster. Enforced by, in order of guarantee:

1. **`automountServiceAccountToken: false`** on the pod — no API credential is mounted, so the pod cannot authenticate to the kube-apiserver **on any CNI**. This is the always-on floor.
2. **Zero-RBAC `fkst-session-runner` SA** — no Role/RoleBinding; never add one.
3. **`networkpolicy.yaml`** — deny ingress, egress to the public internet only (blocks the apiserver, kube-dns, sibling pods, node/metadata). **Only enforced by a real CNI (Calico/Cilium); a no-op on docker-desktop** — set the real pod/service CIDRs in that file.
4. **`baseline` PSS** on the namespace + no privileged/host access, dropped caps, seccomp `RuntimeDefault`. The pod runs as **root** solely so install commands work, boxed by all of the above.
5. **Kata `runtimeClassName`** (strongest tier) — set `FKST_POD_RUNTIME_CLASS` (e.g. `kata`) so both pods run under a sandboxed VM-backed runtime instead of shared-kernel runc. A **prod-only** knob: the nodes must have the Kata runtime installed **and** nested virtualization, and operators must create a cluster-scoped `RuntimeClass` object with that exact name. docker-desktop has neither, so leave it **unset** (empty = runc) locally.

Because the pod cannot reach in-cluster services, `FKST_LLM_BASE_URL` is the **public** LLM host (not the in-cluster `nyxid-backend`).

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

### Applying the opensandbox-mode RBAC

`kustomization.yaml` wires `rbac.yaml` — the **k8s-customized** default. For
**opensandbox** mode, apply `rbac-opensandbox.yaml` INSTEAD (it grants the same
env_store `secrets`/`configmaps` slice with ZERO pod RBAC), and set
`FKST_POD_MODE=opensandbox` + the `FKST_OSB_*` block in `configmap.yaml` (+ the two
`FKST_OSB_*` Secrets in `secret.example.yaml`). Two equivalent ways to swap:

```sh
# Option A — edit kustomization.yaml: replace the `- rbac.yaml` resource entry with
#   - rbac-opensandbox.yaml
# then apply the base as usual:
kubectl apply -k backend/k8s_sample

# Option B — apply the base WITHOUT the k8s-mode RBAC, then the opensandbox RBAC:
kubectl apply -k backend/k8s_sample          # after removing rbac.yaml from resources
kubectl -n <ns> apply -f backend/k8s_sample/rbac-opensandbox.yaml
```

Never apply BOTH RBAC files — `rbac.yaml`'s pod-launcher rules would re-grant the
very pod authority opensandbox mode is meant to withhold. Before flipping the mode
on in a real cluster, complete the
[Infra prerequisites (opensandbox mode)](#infra-prerequisites-opensandbox-mode)
checklist and the [Security-review gate (opensandbox mode)](#security-review-gate-opensandbox-mode).

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

## Infra prerequisites (opensandbox mode)

These are cluster-side facts the **infra team** must satisfy on their OpenSandbox
deployment BEFORE `FKST_POD_MODE=opensandbox` is enabled; fkst cannot control them
from this repo. Each was source-verified against the upstream OpenSandbox server.
Tick a box only after running its verification. Substitute the deployed values for
`$FKST_OSB_BASE_URL`, `$FKST_OSB_API_KEY`, `<fkst-tenant-ns>`, and `<probe-id>`.

- [ ] **1. Multi-tenancy ON — dedicated fkst tenant namespace + dedicated API key. HARD GATE.**
  - Server config carries a `[tenants]` block with a dedicated fkst tenant (its own namespace + its own `api_key`), and `OPENSANDBOX_INSECURE_SERVER` is NOT set (it bypasses auth entirely).
  - A request WITHOUT the key is rejected: `curl -s -o /dev/null -w '%{http_code}' $FKST_OSB_BASE_URL/sandboxes` → **401**.
  - Sandboxes land in the fkst tenant ns, not a shared one: after the probe sandbox (gate 3) exists, `kubectl get pods -n <fkst-tenant-ns> -l fkst-managed=true` lists it.
  > ⚠️ **2026-07-09 live finding:** the current `opensandbox` release in `opensandbox-system` is SINGLE-tenant — sandboxes land in the shared `ornn-cluster` ns, with no `server.api_key` and no `[tenants]` block (and single-tenant mode leaves the sandbox-proxy path unauthenticated). Enabling fkst against it as-is is a **security regression**.

- [ ] **2. Pod template `restartPolicy: Always` + `terminationGracePeriodSeconds: 60`. HARD GATE.**
  In the fkst-tenant BatchSandbox pod template. With `restartPolicy: Never`, the upstream controller (which counts only container *waiting* reasons as failures) reads `state=Pending` forever for an exited engine → completion/crash detection goes blind. Verify by inspecting the tenant's BatchSandbox template — both fields present with these values.

- [ ] **3. `timeout: null` accepted.** Create a probe sandbox with a null timeout → **202**, and confirm it does not auto-expire (BatchSandbox in-tree accepts null; other providers may reject).
  ```sh
  curl -s -o /dev/null -w '%{http_code}' -X POST $FKST_OSB_BASE_URL/sandboxes \
    -H "Authorization: Bearer $FKST_OSB_API_KEY" -H 'Content-Type: application/json' \
    -d '{"image":"<FKST_POD_IMAGE>","timeout":null}'   # expect 202; note the returned <probe-id>
  ```

- [ ] **4. Deprecated plain-text diagnostics-logs endpoint present.** GET it on the probe sandbox → **200, plain text** (the client depends on it; verified across server v0.1.14–v0.2.1).
  ```sh
  curl -s -H "Authorization: Bearer $FKST_OSB_API_KEY" \
    $FKST_OSB_BASE_URL/sandboxes/<probe-id>/diagnostics-logs   # expect 200, plain text
  ```

- [ ] **5. Server version recorded:** `__________` (fill in). The client was written against **v0.2.x** — re-verify gates 3 & 4 on every server upgrade.

- [ ] **6. CNI / egress.**
  - Control-plane → lifecycle server reachable (from a control-plane pod): `curl -s -o /dev/null -w '%{http_code}' $FKST_OSB_BASE_URL/health` → 200.
  - Sandbox egress reaches the three required hosts — from the probe sandbox via execd, `curl -sI https://github.com`, `curl -sI <FKST_LLM_BASE_URL host>`, and `curl -sI <chrono-storage host>` each succeed.
  > **REGRESSION:** OpenSandbox defaults to ALLOW-ALL sandbox egress — looser than the deny-by-default, external-DNS-only NetworkPolicy k8s-customized sessions get (see `networkpolicy.yaml`). The infra team must EITHER configure the egress-policy feature for the fkst tenant OR sign off on the regression:
  > **Allow-all egress sign-off:** `_______________` (name) / `__________` (date).

- [ ] **7. Image pullable from the tenant namespace** (or registry auth configured — the tenant's `imagePullSecrets`, or the create-request `image.auth`). Verify a sandbox on `FKST_POD_IMAGE` starts without `ImagePullBackOff`.

- [ ] **8. Quotas sized for the fleet.** The tenant's sandbox-count + CPU/mem `ResourceQuota` covers the expected concurrent session fleet (each sandbox requests `FKST_OSB_SESSION_CPU` / `FKST_OSB_SESSION_MEMORY`). Verify headroom ≥ peak concurrent sessions.

## E2E smoke runbook (opensandbox mode)

Manual, against a REAL deployment with the prerequisites above satisfied. Each step
lists the expected observation.

| # | Action | Expected observation |
|---|--------|----------------------|
| 1 | Open a trigger issue (the `fkst` label) on an installed repo | A sandbox appears in the fkst tenant ns carrying `fkst-managed=true` metadata |
| 2 | Wait for pickup | The session announces itself on the issue (status comment / label) |
| 3 | Open a work-label issue | The session processes it — a PR appears on the repo |
| 4 | Close the work issue(s) — idle | The session idles and its sandbox is DELETED |
| 5 | Open a new work-label issue | The session REVIVES: a NEW sandbox with the SAME `fkst-session-id` |
| 6 | Close the trigger issue | The session retires and its sandbox is gone |

Plus two probes:

- **Rotation survival** — run a session longer than 1h and confirm a LATE `gh` op in the session logs still succeeds (the credential was rotated in place, not expired).
- **Restart re-push** — kill the sandbox container process via execd and observe the reconcile sweep re-pushing credentials into the fresh container. If container manipulation is unavailable on the deployment, document the equivalent observable instead: a `credentials re-pushed` log line from the sweep after a sandbox restart.

## Security-review gate (opensandbox mode)

Prod enablement of opensandbox mode requires a security review of the CHANGED
credential path. This is PROCESS, not code — run it before the FIRST prod
enablement. In opensandbox mode the credential path differs from k8s-customized:

- The per-session GitHub / App tokens now transit the OpenSandbox **server + proxy as multipart request bodies** (in k8s-customized mode they never leave the cluster's 0400 Secret mounts).
- The **execd token rides the sandbox env**.
- The proxy is an additional exposure surface for those credentials.

Review that path end-to-end, then record the outcome here:

- [ ] **Credential-path security review:** `_______________` (reviewer) / `__________` (date).
