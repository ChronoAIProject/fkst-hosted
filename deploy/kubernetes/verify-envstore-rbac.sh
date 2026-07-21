#!/bin/sh
set -eu

usage() {
  echo "usage: $0 --context CONTEXT [--namespace NAMESPACE] [--service-account NAME]" >&2
  exit 2
}

context=""
namespace="chronoai-fkst"
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
    *) usage ;;
  esac
done

[ -n "$context" ] || usage

subject="system:serviceaccount:${namespace}:${service_account}"
kubectl --context "$context" get namespace "$namespace" >/dev/null

assert_can() {
  verb=$1
  resource=$2
  actual=$(kubectl --context "$context" auth can-i "$verb" "$resource" \
    --as="$subject" --namespace "$namespace" || true)
  if [ "$actual" != "yes" ]; then
    echo "expected yes: $verb $resource (got $actual)" >&2
    exit 1
  fi
  echo "yes  $verb $resource"
}

assert_cannot() {
  verb=$1
  resource=$2
  actual=$(kubectl --context "$context" auth can-i "$verb" "$resource" \
    --as="$subject" --namespace "$namespace" || true)
  if [ "$actual" != "no" ]; then
    echo "expected no: $verb $resource (got $actual)" >&2
    exit 1
  fi
  echo "no   $verb $resource"
}

for resource in configmaps secrets; do
  for verb in create get list update delete; do
    assert_can "$verb" "$resource"
  done
done

for verb in create get list delete; do
  assert_can "$verb" pods
done
assert_can get pods/log
assert_can get pods/status

for resource in configmaps secrets; do
  assert_cannot watch "$resource"
  assert_cannot patch "$resource"
done
assert_cannot watch pods
assert_cannot patch pods
assert_cannot update pods
assert_cannot update pods/status
assert_cannot get deployments.apps

echo "env-store RBAC contract verified for $subject on context $context"
