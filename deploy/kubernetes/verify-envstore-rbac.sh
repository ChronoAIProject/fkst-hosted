#!/bin/sh
set -eu

usage() {
  echo "usage: $0 --context CONTEXT [--namespace NAMESPACE] [--durable-namespace NAMESPACE] [--service-account NAME]" >&2
  exit 2
}

context=""
namespace="chronoai-fkst"
durable_namespace="fkst-recovery-source"
service_account="fkst-ksa"

while [ "$#" -gt 0 ]; do
  case "$1" in
    --context)
      [ "$#" -ge 2 ] || usage
      context=$2
      shift 2
      ;;
    --namespace)
      [ "$#" -ge 2 ] || usage
      namespace=$2
      shift 2
      ;;
    --service-account)
      [ "$#" -ge 2 ] || usage
      service_account=$2
      shift 2
      ;;
    --durable-namespace)
      [ "$#" -ge 2 ] || usage
      durable_namespace=$2
      shift 2
      ;;
    *) usage ;;
  esac
done

[ -n "$context" ] || usage

subject="system:serviceaccount:${namespace}:${service_account}"
kubectl --context "$context" get namespace "$namespace" >/dev/null
kubectl --context "$context" get namespace "$durable_namespace" >/dev/null

assert_can() {
  verb=$1
  resource=$2
  target_namespace=${3:-$namespace}
  actual=$(kubectl --context "$context" auth can-i "$verb" "$resource" \
    --as="$subject" --namespace "$target_namespace" || true)
  if [ "$actual" != "yes" ]; then
    echo "expected yes: $target_namespace $verb $resource (got $actual)" >&2
    exit 1
  fi
  echo "yes  $target_namespace $verb $resource"
}

assert_cannot() {
  verb=$1
  resource=$2
  target_namespace=${3:-$namespace}
  actual=$(kubectl --context "$context" auth can-i "$verb" "$resource" \
    --as="$subject" --namespace "$target_namespace" || true)
  if [ "$actual" != "no" ]; then
    echo "expected no: $target_namespace $verb $resource (got $actual)" >&2
    exit 1
  fi
  echo "no   $target_namespace $verb $resource"
}

for verb in create get list patch delete; do
  assert_can "$verb" pods
done
assert_can get pods/log
assert_can get pods/status

for resource in configmaps secrets; do
  for verb in create get list watch update patch delete; do
    assert_cannot "$verb" "$resource"
  done
done
assert_cannot watch pods
assert_cannot update pods
assert_cannot update pods/status
assert_cannot get deployments.apps

for verb in create get list update delete; do
  assert_can "$verb" secrets "$durable_namespace"
done
for verb in watch patch; do
  assert_cannot "$verb" secrets "$durable_namespace"
done
for verb in create get list watch update patch delete; do
  assert_cannot "$verb" configmaps "$durable_namespace"
done
assert_cannot create pods "$durable_namespace"
assert_cannot get deployments.apps "$durable_namespace"

echo "environment-store RBAC contract verified for $subject on context $context (durable namespace: $durable_namespace)"
