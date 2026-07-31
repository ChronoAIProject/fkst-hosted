#!/bin/sh
set -eu

# Live verification of the durable audit relay.
#
# Everything below is REDACTED by construction: it reads object shapes, Secret
# key NAMES, bounded Prometheus series, and readiness booleans. It never prints a
# Secret value, an event body, a record, a token, an actor, a session id, or a
# repository name, so its output is safe to paste into an incident thread.
#
# The default run is read-only and safe against any environment. `--restart-check
# NAMESPACE` additionally rolls the relay Pod to prove the PVC survives it, which
# briefly stops durable ingress and, in required mode, briefly fails product
# traffic closed — hence the explicit namespace confirmation.

usage() {
  echo "usage: $0 --context CONTEXT [--namespace NAMESPACE] [--timeout DURATION] [--restart-check NAMESPACE]" >&2
  exit 2
}

context=""
namespace="chronoai-fkst"
timeout="5m"
restart_confirmation=""
relay_url="http://fkst-audit-relay.${namespace}.svc.cluster.local"

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
      relay_url="http://fkst-audit-relay.${namespace}.svc.cluster.local"
      shift 2
      ;;
    --timeout)
      [ "$#" -ge 2 ] || usage
      timeout=$2
      shift 2
      ;;
    --restart-check)
      [ "$#" -ge 2 ] || usage
      restart_confirmation=$2
      shift 2
      ;;
    *) usage ;;
  esac
done

[ -n "$context" ] || usage
if [ -n "$restart_confirmation" ] && [ "$restart_confirmation" != "$namespace" ]; then
  echo "refusing --restart-check: confirmation must repeat the target namespace" >&2
  exit 1
fi
kubectl --context "$context" config view --minify --output name >/dev/null

kube() {
  kubectl --context "$context" --namespace "$namespace" "$@"
}

assert_resource() {
  kube get "$1" "$2" --output name >/dev/null
  echo "present  $namespace/$1/$2"
}

# Read one bounded field. `|| true` is deliberate: a missing field must become an
# explicit comparison failure below, not an opaque `set -e` abort.
read_field() {
  kube get "$1" "$2" --output "jsonpath=$3" 2>/dev/null || true
}

# Fetch an in-cluster HTTP body from a control-plane Pod. The relay's
# NetworkPolicy admits the control plane and a labelled Prometheus namespace and
# nothing else, so this is also the only supported way to read it.
relay_get() {
  kube exec deployment/fkst-control-plane -- python3 -c \
    "import sys,urllib.request;sys.stdout.write(urllib.request.urlopen('${relay_url}$1',timeout=10).read().decode())"
}

echo "== objects =="
assert_resource serviceaccount fkst-audit-relay
assert_resource configmap fkst-audit-relay-config
assert_resource externalsecret.external-secrets.io fkst-audit-relay
assert_resource persistentvolumeclaim fkst-audit-relay-data
assert_resource deployment.apps fkst-audit-relay
assert_resource service fkst-audit-relay
assert_resource poddisruptionbudget.policy fkst-audit-relay
assert_resource networkpolicy.networking.k8s.io fkst-audit-relay

echo "== least privilege =="
subject="system:serviceaccount:${namespace}:fkst-audit-relay"
for resource in pods secrets configmaps deployments.apps leases.coordination.k8s.io; do
  answer=$(kubectl --context "$context" auth can-i get "$resource" \
    --as="$subject" --namespace "$namespace" || true)
  if [ "$answer" != "no" ]; then
    echo "the relay identity must hold no Kubernetes API access: get $resource -> $answer" >&2
    exit 1
  fi
  echo "no       $namespace get $resource"
done
automount=$(read_field serviceaccount fkst-audit-relay '{.automountServiceAccountToken}')
[ "$automount" = "false" ] || {
  echo "the relay ServiceAccount must not mount an API token (got '$automount')" >&2
  exit 1
}

echo "== credentials (names only) =="
kube wait --for=condition=Ready externalsecret.external-secrets.io/fkst-audit-relay \
  --timeout="$timeout" >/dev/null
# shellcheck disable=SC2016
keys=$(kube get secret fkst-audit-relay-secret \
  --output go-template='{{range $key, $_ := .data}}{{$key}}{{"\n"}}{{end}}')
for required_key in FKST_AUDIT_RELAY_WRITE_TOKEN FKST_AUDIT_RELAY_READ_TOKEN \
  FKST_POSTHOG_PROJECT_TOKEN FKST_POSTHOG_QUERY_API_KEY; do
  printf '%s\n' "$keys" | grep -Fqx "$required_key" || {
    echo "missing key $required_key in $namespace/secret/fkst-audit-relay-secret" >&2
    exit 1
  }
  # The same name must NOT be a ConfigMap entry: that would put the value in
  # every render, every `kubectl get -o yaml`, and git history.
  present=$(read_field configmap fkst-audit-relay-config "{.data.$required_key}")
  [ -z "$present" ] || {
    echo "$required_key is set in the relay ConfigMap; it must live only in the Secret" >&2
    exit 1
  }
done
echo "keys     4 required names present, 0 in the ConfigMap (values never read)"

echo "== storage =="
phase=$(read_field persistentvolumeclaim fkst-audit-relay-data '{.status.phase}')
[ "$phase" = "Bound" ] || {
  echo "the audit outbox claim is $phase, expected Bound" >&2
  exit 1
}
storage_class=$(read_field persistentvolumeclaim fkst-audit-relay-data '{.spec.storageClassName}')
capacity=$(read_field persistentvolumeclaim fkst-audit-relay-data '{.status.capacity.storage}')
[ -n "$storage_class" ] && [ -n "$capacity" ] || {
  echo "the audit outbox claim must report an explicit storage class and capacity" >&2
  exit 1
}
echo "bound    $capacity on storageClass $storage_class"

echo "== workload =="
kube rollout status deployment/fkst-audit-relay --timeout="$timeout" >/dev/null
replicas=$(read_field deployment.apps fkst-audit-relay '{.spec.replicas}')
strategy=$(read_field deployment.apps fkst-audit-relay '{.spec.strategy.type}')
[ "$replicas" = "1" ] && [ "$strategy" = "Recreate" ] || {
  echo "the relay must be one Recreate replica (got $replicas/$strategy)" >&2
  exit 1
}
echo "rollout  1 replica, Recreate"

echo "== shared configuration =="
relay_grace=$(read_field configmap fkst-audit-relay-config '{.data.FKST_AUDIT_INCOMPLETE_GRACE_SECS}')
plane_grace=$(read_field configmap fkst-control-plane-config '{.data.FKST_AUDIT_INCOMPLETE_GRACE_SECS}')
[ -n "$relay_grace" ] && [ "$relay_grace" = "$plane_grace" ] || {
  echo "FKST_AUDIT_INCOMPLETE_GRACE_SECS disagrees ('$plane_grace' vs '$relay_grace')" >&2
  exit 1
}
mode=$(read_field configmap fkst-control-plane-config '{.data.FKST_AUDIT_DELIVERY_MODE}')
configured_url=$(read_field configmap fkst-control-plane-config '{.data.FKST_AUDIT_RELAY_URL}')
case "$configured_url" in
  *fkst-audit-relay*) : ;;
  *)
    echo "FKST_AUDIT_RELAY_URL does not name the relay Service" >&2
    exit 1
    ;;
esac
echo "shared   grace ${relay_grace}s, delivery mode ${mode:-unset}"

echo "== reachability and isolation =="
ready_body=$(relay_get /ready)
printf '%s' "$ready_body" | grep -Fq '"ready":true' || {
  echo "the relay does not report durable ingress readiness" >&2
  exit 1
}
echo "ready    durable ingress confirmed from a control-plane Pod"
# The cage: a Pod that is not the control plane and not a labelled scraper must
# not reach the port at all. A SUCCESSFUL fetch here is the failure.
if kube exec deployment/fkst-frontend -- \
  wget -T 5 -q -O - "${relay_url}/ready" >/dev/null 2>&1; then
  echo "the relay accepted a connection from the frontend; the NetworkPolicy is not enforced" >&2
  exit 1
fi
echo "blocked  the frontend cannot reach the relay"

echo "== bounded telemetry =="
relay_metrics=$(relay_get /metrics)
for family in fkst_audit_relay_up fkst_audit_relay_ingress_ready fkst_audit_relay_records \
  fkst_audit_relay_oldest_record_age_seconds fkst_audit_relay_db_bytes \
  fkst_audit_relay_dead_letters_total fkst_audit_relay_incomplete_total; do
  printf '%s' "$relay_metrics" | grep -Fq "$family" || {
    echo "the relay exposition is missing $family" >&2
    exit 1
  }
done
for forbidden in actor_id session_id request_id event_id repo_full_name login; do
  if printf '%s' "$relay_metrics" | grep -Fq "$forbidden"; then
    echo "the relay exposition mentions $forbidden; metrics labels must stay bounded" >&2
    exit 1
  fi
done
echo "metrics  7 relay families present, 0 identity tokens"
plane_metrics=$(kube exec deployment/fkst-control-plane -- \
  python3 -c "import sys,urllib.request;sys.stdout.write(urllib.request.urlopen('http://127.0.0.1:8080/metrics',timeout=10).read().decode())")
# Live inventory must stay answerable independently of PostHog and of the relay:
# its series exist whether or not either is healthy.
for family in fkst_audit_required_rejections_total fkst_operations_activity_queries_total \
  fkst_operations_sandbox_inventory_requests_total fkst_session_access_registry_generation_state; do
  printf '%s' "$plane_metrics" | grep -Fq "$family" || {
    echo "the control-plane exposition is missing $family" >&2
    exit 1
  }
done
echo "metrics  4 control-plane audit families present"

if [ -n "$restart_confirmation" ]; then
  echo "== restart and volume persistence =="
  before=$(printf '%s' "$relay_metrics" | awk '/^fkst_audit_relay_records\{/ { total += $2 } END { print total + 0 }')
  kube rollout restart deployment/fkst-audit-relay >/dev/null
  kube rollout status deployment/fkst-audit-relay --timeout="$timeout" >/dev/null
  after=$(relay_get /metrics | awk '/^fkst_audit_relay_records\{/ { total += $2 } END { print total + 0 }')
  if [ "$after" -lt "$before" ]; then
    echo "records dropped from $before to $after across a restart; the outbox is not persistent" >&2
    exit 1
  fi
  echo "restart  $before records before, $after after (no loss)"
fi

echo "audit relay contract verified in $namespace on context $context"
