#!/bin/sh
set -eu

usage() {
  echo "usage: $0 --context CONTEXT [--overlay PATH] [--timeout DURATION] [--preflight-only]" >&2
  exit 2
}

context=""
timeout="10m"
preflight_only="false"
script_dir=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
overlay="$script_dir/overlays/local"

while [ "$#" -gt 0 ]; do
  case "$1" in
    --context)
      [ "$#" -ge 2 ] || usage
      context=$2
      shift 2
      ;;
    --overlay)
      [ "$#" -ge 2 ] || usage
      overlay=$2
      shift 2
      ;;
    --timeout)
      [ "$#" -ge 2 ] || usage
      timeout=$2
      shift 2
      ;;
    --preflight-only)
      preflight_only="true"
      shift
      ;;
    *) usage ;;
  esac
done

[ -n "$context" ] || usage
[ -f "$overlay/kustomization.yaml" ] || {
  echo "overlay has no kustomization.yaml: $overlay" >&2
  exit 1
}
[ -f "$overlay/secrets/kustomization.yaml" ] || {
  echo "overlay must provide an independently renderable secrets/ kustomization" >&2
  exit 1
}

kubectl --context "$context" config view --minify --output name >/dev/null
kubectl --context "$context" get namespace opensandbox-system >/dev/null
kubectl --context "$context" get customresourcedefinition.apiextensions.k8s.io \
  externalsecrets.external-secrets.io >/dev/null
kubectl --context "$context" get customresourcedefinition.apiextensions.k8s.io \
  secretstores.external-secrets.io >/dev/null

rendered=$(mktemp "${TMPDIR:-/tmp}/fkst-restore.XXXXXX")
trap 'rm -f "$rendered"' EXIT HUP INT TERM
kubectl --context "$context" kustomize "$overlay" >"$rendered"
if awk '$1 == "kind:" && $2 == "Secret" { found = 1 } END { exit !found }' "$rendered"; then
  echo "refusing to restore: rendered IaC contains a plaintext Kubernetes Secret" >&2
  exit 1
fi

if [ "$preflight_only" = "true" ]; then
  kubectl --context "$context" apply -k "$overlay" --dry-run=server >/dev/null
  echo "restore preflight passed on context $context"
  exit 0
fi

echo "phase 1/5: namespace and security policy"
kubectl --context "$context" apply -f "$script_dir/base/namespace.yaml"
kubectl --context "$context" apply -f "$script_dir/base/guardrails.yaml"

echo "phase 2/5: service identity, RBAC, and external-secret bindings"
kubectl --context "$context" apply -f "$script_dir/base/service-accounts.yaml"
kubectl --context "$context" apply -f "$script_dir/base/env-store-rbac.yaml"
kubectl --context "$context" apply -f "$script_dir/base/leader-election-rbac.yaml"
kubectl --context "$context" apply -f "$script_dir/base/opensandbox-template.yaml"
kubectl --context "$context" apply -k "$overlay/secrets"

kubectl --context "$context" --namespace chronoai-fkst wait \
  --for=condition=Ready secretstore/fkst-external-secrets --timeout="$timeout"
kubectl --context "$context" --namespace opensandbox-system wait \
  --for=condition=Ready secretstore/fkst-external-secrets --timeout="$timeout"
for external_secret in fkst-control-plane opensandbox-fkst-api-key fkst-ingress-tls; do
  kubectl --context "$context" --namespace chronoai-fkst wait \
    --for=condition=Ready "externalsecret/$external_secret" --timeout="$timeout"
done
kubectl --context "$context" --namespace opensandbox-system wait \
  --for=condition=Ready externalsecret/opensandbox-api-key --timeout="$timeout"

echo "phase 3/5: durable environment-profile backend dependency"
echo "external credential records are ready; the selected overlay must provide the I7 durable environment store"

echo "phase 4/5: control plane, frontend, services, and route"
kubectl --context "$context" apply -k "$overlay"
kubectl --context "$context" --namespace chronoai-fkst rollout status \
  deployment/fkst-control-plane --timeout="$timeout"
kubectl --context "$context" --namespace chronoai-fkst rollout status \
  deployment/fkst-frontend --timeout="$timeout"

echo "phase 5/5: startup recovery and post-restore verification"
"$script_dir/verify-namespace.sh" --context "$context" --timeout "$timeout"
echo "fkst namespace restore converged on context $context"
