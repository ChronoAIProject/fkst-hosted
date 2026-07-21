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
| `opensandbox/server-values.yaml` | Canonical FKST tenant, API-key file, and BatchSandbox-template integration for the lifecycle-server chart. |
| `restore-namespace.sh` | Ordered, non-destructive namespace convergence. It requires an explicit context and waits for secret materialization before workloads. |
| `verify-namespace.sh` | Redacted live verification of security, RBAC, ExternalSecret, rollout, route, and recovery-readiness contracts. |
| `validate-manifests.sh` | Deterministic render and structural/security policy checks; also runs shellcheck and kubeconform when installed. |

The base uses one control-plane replica and `Recreate` rollout semantics. Do not
increase the replica count until the I8 Lease implementation is enabled and its
failover test passes. The Lease Role is provisioned ahead of that switch so a
namespace restore does not require an out-of-band RBAC edit.

## External state contract

The `fkst-external-secrets` SecretStore is environment-owned. A cloud overlay
must patch it to the reviewed provider and bind provider workload identity to
the service account used by that provider. The checked-in local overlay is only
a same-cluster stand-in for disaster testing; it is not a production secret
manager.

The provider exposes three logical records:

| Remote record | Materialized Secret | Required key names |
|---|---|---|
| `fkst-control-plane` | `chronoai-fkst/fkst-control-plane-secret` | `FKST_LLM_API_KEY`, `FKST_OSB_EXECD_TOKEN_SEED`, `FKST_GITHUB_APP_ID`, `FKST_GITHUB_APP_PRIVATE_KEY_PEM`, `FKST_GITHUB_APP_SLUG`, `FKST_GITHUB_APP_WEBHOOK_SECRET`, `FKST_GITHUB_OAUTH_CLIENT_SECRET` |
| `fkst-opensandbox-tenant` | `chronoai-fkst/opensandbox-fkst-api-key` and `opensandbox-system/opensandbox-api-key` | `opensandbox-fkst-api-key` |
| `fkst-ingress-tls` | `chronoai-fkst/fkst-ingress-tls` | `tls.crt`, `tls.key` |

Optional broader-OAuth and log-storage deployments put
`FKST_GITHUB_BROADER_OAUTH_CLIENT_SECRET` and `FKST_NYXID_CLIENT_SECRET` in the
control-plane record. Their non-secret client IDs, endpoints, and bucket names
belong in the environment ConfigMap patch. ExternalSecret status and Secret key
names are safe to inspect; Secret values are not.

The backend currently stores named environment profiles as ConfigMap/Secret
pairs in `chronoai-fkst`. These manifests do not claim that data is durable.
I7 must supply the namespace-independent `EnvironmentProfileStore` and its
connectivity before the full namespace-loss acceptance criterion can pass.

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
  --from-literal=FKST_GITHUB_OAUTH_CLIENT_SECRET='<local value>'

kubectl --context kind-opensandbox-local --namespace fkst-recovery-source \
  create secret generic fkst-opensandbox-tenant \
  --from-literal=opensandbox-fkst-api-key='<local value>'

kubectl --context kind-opensandbox-local --namespace fkst-recovery-source \
  create secret tls fkst-ingress-tls \
  --cert='<local certificate file>' --key='<local key file>'
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

The script applies namespace/security policy, identity/RBAC/ExternalSecrets,
waits for materialized credentials, applies services and routes, waits for both
rollouts, and finally requires `/ready` to report a completed startup resync.
It never creates plaintext Secrets, changes kube context, or performs deletion.

Run live verification independently at any time:

```bash
deploy/kubernetes/verify-namespace.sh --context kind-opensandbox-local
```

`verify-envstore-rbac.sh` remains the focused least-privilege check. It proves
the environment store and validation-pod grants as well as representative
denials:

```bash
deploy/kubernetes/verify-envstore-rbac.sh --context kind-opensandbox-local
```

## Environment overlays

A deployment overlay should reference `base/` and `external-secrets/` rather
than copy their objects. It must patch, at minimum:

- immutable image references for the control plane and frontend;
- public API/frontend hosts and TLS record;
- LLM endpoint/model and GitHub App bot login/client ID;
- access model, allowed GitHub logins, and global-admin logins;
- optional storage/NyxID endpoints and identities;
- the External Secret provider/store references and remote record identifiers;
- provider-specific Workload Identity annotations;
- runtime class, placement, resources, and replica policy appropriate to the
  environment.

Keep provider credentials, Secret resources, private keys, bearer tokens, and
encoded secret values outside Git. A production overlay belongs in the reviewed
infrastructure repository when that environment's ownership boundary is
separate from this application repository.
