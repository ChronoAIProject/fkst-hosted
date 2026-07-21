#!/bin/sh
set -eu

usage() {
  echo "usage: $0 --context CONTEXT [--timeout DURATION] [--durable-namespace NAMESPACE] [--sentinel-user-id ID --sentinel-name NAME --sentinel-content-hash SHA256 --sentinel-secret-keys CSV]" >&2
  exit 2
}

context=""
timeout="5m"
namespace="chronoai-fkst"
durable_namespace="fkst-recovery-source"
sentinel_user_id=""
sentinel_name=""
sentinel_content_hash=""
sentinel_secret_keys=""
sentinel_fields=0
script_dir=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)

while [ "$#" -gt 0 ]; do
  case "$1" in
    --context)
      [ "$#" -ge 2 ] || usage
      context=$2
      shift 2
      ;;
    --timeout)
      [ "$#" -ge 2 ] || usage
      timeout=$2
      shift 2
      ;;
    --durable-namespace)
      [ "$#" -ge 2 ] || usage
      durable_namespace=$2
      shift 2
      ;;
    --sentinel-user-id)
      [ "$#" -ge 2 ] || usage
      sentinel_user_id=$2
      sentinel_fields=$((sentinel_fields + 1))
      shift 2
      ;;
    --sentinel-name)
      [ "$#" -ge 2 ] || usage
      sentinel_name=$2
      sentinel_fields=$((sentinel_fields + 1))
      shift 2
      ;;
    --sentinel-content-hash)
      [ "$#" -ge 2 ] || usage
      sentinel_content_hash=$2
      sentinel_fields=$((sentinel_fields + 1))
      shift 2
      ;;
    --sentinel-secret-keys)
      [ "$#" -ge 2 ] || usage
      sentinel_secret_keys=$2
      sentinel_fields=$((sentinel_fields + 1))
      shift 2
      ;;
    *) usage ;;
  esac
done

[ -n "$context" ] || usage
[ "$sentinel_fields" -eq 0 ] || [ "$sentinel_fields" -eq 4 ] || usage
kubectl --context "$context" config view --minify --output name >/dev/null

assert_resource() {
  resource=$1
  name=$2
  resource_namespace=${3:-$namespace}
  kubectl --context "$context" --namespace "$resource_namespace" get \
    "$resource" "$name" --output name >/dev/null
  echo "present  $resource_namespace/$resource/$name"
}

assert_secret_keys() {
  secret=$1
  secret_namespace=$2
  shift 2
  # shellcheck disable=SC2016
  keys=$(kubectl --context "$context" --namespace "$secret_namespace" get \
    secret "$secret" --output go-template='{{range $key, $_ := .data}}{{$key}}{{"\n"}}{{end}}')
  for required_key in "$@"; do
    if ! printf '%s\n' "$keys" | grep -Fqx "$required_key"; then
      echo "missing key $required_key in $secret_namespace/secret/$secret" >&2
      exit 1
    fi
  done
  echo "keys     $secret_namespace/secret/$secret ($# required names present; values redacted)"
}

enforce=$(kubectl --context "$context" get namespace "$namespace" \
  --output jsonpath='{.metadata.labels.pod-security\.kubernetes\.io/enforce}')
[ "$enforce" = "baseline" ] || {
  echo "namespace Pod Security enforce label is $enforce, expected baseline" >&2
  exit 1
}

for service_account in sandbox-runner fkst-ksa; do
  assert_resource serviceaccount "$service_account"
done
assert_resource limitrange sandbox-limits
assert_resource resourcequota sandbox-quota
assert_resource networkpolicy.networking.k8s.io sandbox-lockdown
assert_resource role.rbac.authorization.k8s.io fkst-control-plane-envstore
assert_resource role.rbac.authorization.k8s.io fkst-control-plane-leader-election
assert_resource rolebinding.rbac.authorization.k8s.io fkst-control-plane-envstore
assert_resource rolebinding.rbac.authorization.k8s.io fkst-control-plane-leader-election
assert_resource role.rbac.authorization.k8s.io fkst-control-plane-durable-envstore "$durable_namespace"
assert_resource rolebinding.rbac.authorization.k8s.io fkst-control-plane-durable-envstore "$durable_namespace"
assert_resource configmap fkst-control-plane-config
assert_resource configmap opensandbox-batchsandbox-template opensandbox-system
assert_resource ingress.networking.k8s.io fkst
assert_resource poddisruptionbudget.policy fkst-control-plane
assert_resource poddisruptionbudget.policy fkst-frontend

for external_secret in fkst-control-plane opensandbox-fkst-api-key fkst-ingress-tls; do
  kubectl --context "$context" --namespace "$namespace" wait \
    --for=condition=Ready "externalsecret/$external_secret" --timeout="$timeout" >/dev/null
done
kubectl --context "$context" --namespace opensandbox-system wait \
  --for=condition=Ready externalsecret/opensandbox-api-key --timeout="$timeout" >/dev/null

assert_secret_keys fkst-control-plane-secret "$namespace" \
  FKST_LLM_API_KEY FKST_OSB_EXECD_TOKEN_SEED FKST_GITHUB_APP_ID \
  FKST_GITHUB_APP_PRIVATE_KEY_PEM FKST_GITHUB_APP_SLUG \
  FKST_GITHUB_APP_WEBHOOK_SECRET FKST_GITHUB_OAUTH_CLIENT_SECRET \
  FKST_ENV_STORE_ENCRYPTION_KEY
assert_secret_keys opensandbox-fkst-api-key "$namespace" opensandbox-fkst-api-key
assert_secret_keys opensandbox-api-key opensandbox-system opensandbox-fkst-api-key
assert_secret_keys fkst-ingress-tls "$namespace" tls.crt tls.key

"$script_dir/verify-envstore-rbac.sh" --context "$context" \
  --durable-namespace "$durable_namespace"
subject="system:serviceaccount:${namespace}:fkst-ksa"
for verb in create get list watch update patch; do
  actual=$(kubectl --context "$context" auth can-i "$verb" leases.coordination.k8s.io \
    --as="$subject" --namespace "$namespace" || true)
  [ "$actual" = "yes" ] || {
    echo "expected yes: $verb leases.coordination.k8s.io (got $actual)" >&2
    exit 1
  }
done
actual=$(kubectl --context "$context" auth can-i delete leases.coordination.k8s.io \
  --as="$subject" --namespace "$namespace" || true)
[ "$actual" = "no" ] || {
  echo "expected no: delete leases.coordination.k8s.io (got $actual)" >&2
  exit 1
}
actual=$(kubectl --context "$context" auth can-i patch pods \
  --as="$subject" --namespace "$namespace" || true)
[ "$actual" = "yes" ] || {
  echo "expected yes: patch pods for leader Service routing (got $actual)" >&2
  exit 1
}

kubectl --context "$context" --namespace "$namespace" rollout status \
  deployment/fkst-control-plane --timeout="$timeout" >/dev/null
kubectl --context "$context" --namespace "$namespace" rollout status \
  deployment/fkst-frontend --timeout="$timeout" >/dev/null

replicas=$(kubectl --context "$context" --namespace "$namespace" get \
  deployment fkst-control-plane --output jsonpath='{.spec.replicas}')
available=$(kubectl --context "$context" --namespace "$namespace" get \
  deployment fkst-control-plane --output jsonpath='{.status.availableReplicas}')
[ "$replicas" = "2" ] && [ "$available" = "2" ] || {
  echo "control plane must have two healthy replicas (desired=$replicas available=$available)" >&2
  exit 1
}

# The Pods are health-ready for Deployment convergence, while exactly one
# resync-complete Lease holder publishes the additional Service selector label.
attempt=0
leader_count=0
while [ "$attempt" -lt 60 ]; do
  leader_count=$(kubectl --context "$context" --namespace "$namespace" get pods \
    --selector='app.kubernetes.io/name=fkst-control-plane,fkst.chronoai.io/leader-serving=true' \
    --output name | wc -l | tr -d ' ')
  [ "$leader_count" = "1" ] && break
  attempt=$((attempt + 1))
  sleep 1
done
[ "$leader_count" = "1" ] || {
  echo "expected exactly one Service-published leader, got $leader_count" >&2
  exit 1
}

selected_pod=$(kubectl --context "$context" --namespace "$namespace" get pods \
  --selector='app.kubernetes.io/name=fkst-control-plane,fkst.chronoai.io/leader-serving=true' \
  --output jsonpath='{.items[0].metadata.name}')
lease_holder=$(kubectl --context "$context" --namespace "$namespace" get \
  lease.coordination.k8s.io fkst-control-plane-reconciler \
  --output jsonpath='{.spec.holderIdentity}')
[ "$selected_pod" = "$lease_holder" ] || {
  echo "Service-selected pod does not match Lease holder" >&2
  exit 1
}
lease_transitions=$(kubectl --context "$context" --namespace "$namespace" get \
  lease.coordination.k8s.io fkst-control-plane-reconciler \
  --output jsonpath='{.spec.leaseTransitions}')
case "$lease_transitions" in
  ''|*[!0-9]*) echo "Lease transition history is missing or invalid" >&2; exit 1 ;;
esac
echo "leader   $lease_holder (one Service endpoint; Lease transitions=$lease_transitions)"

ready=$(kubectl --context "$context" --namespace "$namespace" exec \
  deployment/fkst-frontend -- wget -qO- http://fkst-control-plane/ready)
printf '%s\n' "$ready" | grep -Eq '"status"[[:space:]]*:[[:space:]]*"ready"'
printf '%s\n' "$ready" | grep -Eq \
  '"startup_resync_complete"[[:space:]]*:[[:space:]]*true'
kubectl --context "$context" --namespace "$namespace" exec \
  deployment/fkst-frontend -- wget -qO- http://127.0.0.1:8080/ >/dev/null

if [ "$sentinel_fields" -eq 4 ]; then
  sentinel="fkst-env-${sentinel_user_id}-${sentinel_name}"
  assert_resource secret "$sentinel" "$durable_namespace"
  assert_secret_keys "$sentinel" "$durable_namespace" nonce ciphertext
  actual_hash=$(kubectl --context "$context" --namespace "$durable_namespace" get \
    secret "$sentinel" --output go-template='{{index .metadata.annotations "fkst.chrono-ai.fun/content-hash"}}')
  [ "$actual_hash" = "$sentinel_content_hash" ] || {
    echo "durable environment sentinel content hash differs" >&2
    exit 1
  }
  actual_keys_json=$(kubectl --context "$context" --namespace "$durable_namespace" get \
    secret "$sentinel" --output go-template='{{index .metadata.annotations "fkst.chrono-ai.fun/env-secret-keys"}}')
  actual_keys=$(printf '%s' "$actual_keys_json" | tr -d '[]" ' | tr ',' '\n' | \
    sed '/^$/d' | LC_ALL=C sort | paste -sd, -)
  expected_keys=$(printf '%s' "$sentinel_secret_keys" | tr ',' '\n' | \
    sed 's/^[[:space:]]*//;s/[[:space:]]*$//;/^$/d' | LC_ALL=C sort | paste -sd, -)
  [ "$actual_keys" = "$expected_keys" ] || {
    echo "durable environment sentinel secret-key inventory differs" >&2
    exit 1
  }
  echo "sentinel durable environment hash and secret-key inventory verified (values redacted)"
fi

echo "namespace contract and recovery readiness verified on context $context"
