#!/bin/sh
set -eu

usage() {
  echo "usage: $0 --context CONTEXT [--overlay PATH] [--steady-overlay PATH] [--durable-namespace NAMESPACE] [--timeout DURATION]" >&2
  exit 2
}

context=""
timeout="10m"
durable_namespace="fkst-recovery-source"
script_dir=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
migration_overlay="$script_dir/overlays/local-migration"
steady_overlay="$script_dir/overlays/local"

while [ "$#" -gt 0 ]; do
  case "$1" in
    --context)
      [ "$#" -ge 2 ] || usage
      context=$2
      shift 2
      ;;
    --overlay)
      [ "$#" -ge 2 ] || usage
      migration_overlay=$2
      shift 2
      ;;
    --steady-overlay)
      [ "$#" -ge 2 ] || usage
      steady_overlay=$2
      shift 2
      ;;
    --durable-namespace)
      [ "$#" -ge 2 ] || usage
      durable_namespace=$2
      shift 2
      ;;
    --timeout)
      [ "$#" -ge 2 ] || usage
      timeout=$2
      shift 2
      ;;
    *) usage ;;
  esac
done

[ -n "$context" ] || usage
for overlay in "$migration_overlay" "$steady_overlay"; do
  [ -f "$overlay/kustomization.yaml" ] || {
    echo "overlay has no kustomization.yaml: $overlay" >&2
    exit 1
  }
done

kubectl --context "$context" config view --minify --output name >/dev/null
kubectl --context "$context" apply -k "$migration_overlay"
kubectl --context "$context" --namespace chronoai-fkst wait \
  --for=condition=Ready externalsecret/fkst-control-plane --timeout="$timeout"
kubectl --context "$context" --namespace chronoai-fkst rollout restart \
  deployment/fkst-control-plane
kubectl --context "$context" --namespace chronoai-fkst rollout status \
  deployment/fkst-control-plane --timeout="$timeout"

selector="app.kubernetes.io/component=user-env"
remaining_config_maps=$(kubectl --context "$context" --namespace chronoai-fkst get \
  configmaps --selector "$selector" --output name)
remaining_secrets=$(kubectl --context "$context" --namespace chronoai-fkst get \
  secrets --selector "$selector" --output name)
if [ -n "$remaining_config_maps" ] || [ -n "$remaining_secrets" ]; then
  echo "legacy environment objects remain after migration; keeping temporary RBAC for retry" >&2
  exit 1
fi

kubectl --context "$context" apply -k "$steady_overlay"
kubectl --context "$context" --namespace chronoai-fkst patch \
  configmap fkst-control-plane-config --type=merge \
  --patch='{"data":{"FKST_ENV_STORE_LEGACY_NAMESPACE":null}}' >/dev/null
kubectl --context "$context" --namespace chronoai-fkst rollout restart \
  deployment/fkst-control-plane
kubectl --context "$context" --namespace chronoai-fkst rollout status \
  deployment/fkst-control-plane --timeout="$timeout"
legacy_namespace=$(kubectl --context "$context" --namespace chronoai-fkst get \
  configmap fkst-control-plane-config \
  --output jsonpath='{.data.FKST_ENV_STORE_LEGACY_NAMESPACE}')
[ -z "$legacy_namespace" ] || {
  echo "steady-state ConfigMap still enables legacy migration" >&2
  exit 1
}

kubectl --context "$context" delete -k "$script_dir/migrations" --ignore-not-found
"$script_dir/verify-envstore-rbac.sh" --context "$context" \
  --durable-namespace "$durable_namespace"
echo "legacy environment profiles migrated and steady-state RBAC converged on context $context"
