# Kubernetes deployment sources

`base/` is the canonical, checked-in source for namespace-scoped fkst-hosted
resources. Apply it only with an explicit Kubernetes context:

```bash
kubectl --context <context> apply -k deploy/kubernetes/base
```

The base currently establishes the environment-store and validation-runtime RBAC
contract. `verify-envstore-rbac.sh` checks the live grants and representative
denials; it refuses to run unless `--context` is provided:

```bash
deploy/kubernetes/verify-envstore-rbac.sh --context <context>
```

Do not commit Secret objects or provider credentials here. Deployment-specific
secret material must be delivered by an external secret provider or a local,
untracked overlay.
