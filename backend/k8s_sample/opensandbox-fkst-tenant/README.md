# OpenSandbox fkst tenant — infra runbook + sample manifests

The exact cluster-side state fkst-hosted's `FKST_POD_MODE=opensandbox` expects in
production, as copy-adaptable samples. **Audience: the infra team** (none of this
is applied by fkst-hosted's own deploy) plus future contributors who need the
verified production facts. Everything here was verified against the deployed
OpenSandbox release (server `v0.2.1`, controller `v0.2.0`, execd `v1.0.20`;
sources at tag `server/v0.2.1` of `alibaba/OpenSandbox`) on 2026-07-10.

## Why a DEDICATED server instance

OpenSandbox server v0.2.1 is single-tenant per instance:

- **one API key** — a single string (`opensandbox_server/middleware/auth.py::_load_api_keys`);
  there is no key→tenant mapping and the create API has no namespace field, so a
  caller can never choose where its sandboxes land;
- **one target namespace** — `[kubernetes].namespace` in `config.toml`;
- **one BatchSandbox pod template** — `batchsandbox_template_file`, global to the
  instance.

The existing production instance (`opensandbox-server` in `opensandbox-system`)
pins `namespace = "chronoai-chrono-sandbox"` and a template with
`restartPolicy: Never` / `terminationGracePeriodSeconds: 30` on the
scale-from-zero gVisor spot pool — correct for chrono-sandbox's short-lived
one-shot executions, **incompatible with fkst sessions**, which hard-require
`restartPolicy: Always` + `terminationGracePeriodSeconds: 60` (long-lived
devloops; the credential-heal path re-pushes creds after an IN-PLACE container
restart, which `Never` makes impossible). Since the template is per-instance
global config, fkst needs its own instance — which also buys key separation
(chrono-sandbox and fkst can never list/delete each other's sandboxes) and
per-tenant quotas.

## Topology

| Piece | Value |
|---|---|
| Server Deployment + Service | `opensandbox-server-fkst` in `opensandbox-system` (same chart/image `opensandbox/server:v0.2.1` as the existing instance) |
| Controller + CRDs | **shared** — the cluster-wide `opensandbox-controller-manager` (v0.2.0) and `*.sandbox.opensandbox.io` CRDs are namespace-agnostic; only the server instance pins the tenant namespace |
| Sandbox namespace | `chronoai-fkst-sandbox` (`namespace.yaml`). The empty pre-existing `chronoai-opensandbox` namespace is an acceptable substitute if infra prefers reuse — **open choice for infra**; adjust every manifest here if taken |
| fkst-hosted backend | stays in `chronoai-fkst` — never co-located with sandboxes; all exec traffic rides the server proxy |
| Base URL the backend uses | `FKST_OSB_BASE_URL=http://opensandbox-server-fkst.opensandbox-system.svc.cluster.local` (in-cluster ClusterIP; the public proxy-fronted URL 504s at ~60s while a spot cold-start create legitimately takes 1–3 min) |

## Key distribution (GCP Secret Manager → Secrets Store CSI)

Two NEW secrets in GCP Secret Manager, project `chronoai-501608`:

| Secret | Consumed by | How |
|---|---|---|
| `opensandbox-fkst-api-key` | the fkst server instance | CSI mount + command wrapper, same as the existing instance: `export OPENSANDBOX_SERVER_API_KEY=$(cat /var/secrets/opensandbox-fkst-api-key) && exec opensandbox-server --config …` |
| `opensandbox-fkst-api-key` | the fkst-hosted backend | CSI mount (`secretproviderclass-backend.yaml`) + `FKST_OSB_API_KEY_FILE=/var/secrets/opensandbox-fkst-api-key` |
| `fkst-osb-execd-token-seed` | the fkst-hosted backend only | CSI mount + `FKST_OSB_EXECD_TOKEN_SEED_FILE=/var/secrets/fkst-osb-execd-token-seed` |

IAM: grant `roles/secretmanager.secretAccessor` (permission
`secretmanager.versions.access`) on those two secrets to the consuming KSAs via
GKE Workload Identity Federation principals, e.g. for the backend:

```
principal://iam.googleapis.com/projects/<PROJECT_NUMBER>/locations/global/workloadIdentityPools/<PROJECT_ID>.svc.id.goog/subject/ns/chronoai-fkst/sa/fkst-control-plane
```

and analogously `ns/opensandbox-system/sa/<fkst server KSA>` for the server's
copy. **Never reuse the chrono-sandbox key** (`opensandbox-api-key`) — separate
keys are the tenant isolation boundary.

## Node-pool decision (explicit infra sign-off)

The chrono template targets the scale-from-zero gVisor **spot** pool. fkst
sessions are long-lived: a spot preemption kills the session pod mid-work
(`restartPolicy` does not survive node loss) and the fkst reconciler respawns it
from scratch, losing in-flight work (branch state, running builds).

**Recommendation: an on-demand (non-spot) gVisor pool for this tenant.** If
infra accepts spot economics instead, record that respawn-and-redo is the
accepted behavior. The template's `nodeSelector`/`toleration` pin gVisor either
way; pool choice is a taint/selector concern on the pool itself.

## Security notes (verified against the deployed sources)

- The server's proxy route `/(v1/)sandboxes/{id}/proxy/{port}/…` is
  **API-key-exempt** in the auth middleware (`middleware/auth.py` skips auth for
  that exact path shape). The per-session execd token
  (`X-EXECD-ACCESS-TOKEN`, enforced by execd v1.0.20 against its
  `EXECD_ACCESS_TOKEN` env — empty or mismatched → 401) is therefore the ONLY
  auth on the exec plane. Consequence: restrict who can reach the fkst server
  Service at all — `networkpolicy-server-ingress.yaml`.
- Sandbox egress is locked to public internet + kube-dns by
  `networkpolicy-sandbox-lockdown.yaml` (RFC1918 + the GCP metadata IP
  `169.254.169.254` blocked). Everything an in-sandbox fkst engine dials
  (GitHub, the LLM endpoint, chrono-storage) MUST be a public URL.
- Sandboxes run under the unbound `sandbox-runner` SA with
  `automountServiceAccountToken: false` and gVisor — no k8s/GCP ambient
  authority inside a session.

## Infra verification checklist

Run from a pod in `chronoai-fkst` (each item: command → expected result).
`BASE=http://opensandbox-server-fkst.opensandbox-system.svc.cluster.local`,
`KEY=$(cat /var/secrets/opensandbox-fkst-api-key)`.

1. **Health (auth-exempt, root — not under /v1):**
   `curl -s -o /dev/null -w '%{http_code}' $BASE/health` → `200`.
2. **API-key gate:**
   `curl -s -o /dev/null -w '%{http_code}' $BASE/v1/sandboxes` → `401`.
3. **Create with `timeout: null` accepted** (null BYPASSES the TTL cap — verified
   in `services/validators.py::ensure_timeout_within_limit`, which returns early
   on null, and the batchsandbox provider pops `expireTime`):
   ```sh
   curl -s -o /dev/null -w '%{http_code}' -X POST $BASE/v1/sandboxes \
     -H "OPEN-SANDBOX-API-KEY: $KEY" -H 'Content-Type: application/json' \
     -d '{"image":{"uri":"ubuntu"},"entrypoint":["tail","-f","/dev/null"],
          "resourceLimits":{"cpu":"250m","memory":"256Mi"},"timeout":null}'
   ```
   → `202`. Note the returned `id` as `<probe-id>`.
4. **Pod placement + template gates:** the pod `<probe-id>-0` exists in
   `chronoai-fkst-sandbox` with `restartPolicy: Always`,
   `terminationGracePeriodSeconds: 60`, `runtimeClassName: gvisor`,
   `serviceAccountName: sandbox-runner`, `automountServiceAccountToken: false`:
   `kubectl -n chronoai-fkst-sandbox get pod <probe-id>-0 -o yaml | grep -E 'restartPolicy|terminationGrace|runtimeClassName|serviceAccountName|automountService'`.
5. **Deprecated plain-text diagnostics logs present** (NO `scope` param — `scope`
   selects the stable JSON API, which answers `501`):
   `curl -s -H "OPEN-SANDBOX-API-KEY: $KEY" "$BASE/v1/sandboxes/<probe-id>/diagnostics/logs?tail=5"`
   → `200`, plain text.
6. **execd token enforced** (the proxy route needs no API key — that is the
   point of this check):
   `curl -s -o /dev/null -w '%{http_code}' -H "X-EXECD-ACCESS-TOKEN: wrong" "$BASE/v1/sandboxes/<probe-id>/proxy/44772/files/info?path=/"`
   → `401`.
7. **Cleanup:** `curl -s -o /dev/null -w '%{http_code}' -X DELETE -H "OPEN-SANDBOX-API-KEY: $KEY" $BASE/v1/sandboxes/<probe-id>` → `204`.

## Files here

| File | What |
|---|---|
| `namespace.yaml` | the dedicated sandbox namespace |
| `serviceaccount-sandbox-runner.yaml` | unbound sandbox SA |
| `server-config.toml` | the fkst server instance's `config.toml` |
| `batchsandbox-template.yaml` | the fkst pod template (Always / grace 60) |
| `networkpolicy-sandbox-lockdown.yaml` | sandbox ingress/egress guardrail |
| `networkpolicy-server-ingress.yaml` | who may call the fkst server |
| `resourcequota.yaml` | fleet quota sample |
| `secretproviderclass-backend.yaml` | CSI mapping for the backend's two secrets |
| `backend-deployment-patch.yaml` | CSI volume + `FKST_OSB_*` env for the backend |
