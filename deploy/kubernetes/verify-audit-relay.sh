#!/bin/sh
set -eu

# Live verification of the durable audit relay.
#
# Everything below is REDACTED by construction: it reads object shapes, Secret
# key NAMES, bounded Prometheus series, and readiness booleans. It never prints a
# Secret value, an event body, a record, a token, an actor, a session id, or a
# repository name, so its output is safe to paste into an incident thread.
#
# The default run is read-only and safe against any environment. Two opt-in
# drills mutate the relay Deployment and are therefore gated on repeating the
# target namespace, because both briefly stop durable ingress and, in required
# mode, briefly fail product traffic closed:
#
#   --restart-check NAMESPACE  rolls the Pod, proving the PVC survives it.
#   --outage-drill NAMESPACE   scales the relay to zero and back, proving live
#                              sandbox inventory is independent of it, that the
#                              outage drains rather than loses, and that a
#                              PostHog that cannot be reached never takes durable
#                              ingress down.
#
# Run the drills in a DISPOSABLE cluster. Neither is safe to run casually against
# a deployment serving required-mode traffic.

usage() {
  echo "usage: $0 --context CONTEXT [--namespace NAMESPACE] [--timeout DURATION] [--restart-check NAMESPACE] [--outage-drill NAMESPACE]" >&2
  exit 2
}

context=""
namespace="chronoai-fkst"
timeout="5m"
restart_confirmation=""
outage_confirmation=""
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
    --outage-drill)
      [ "$#" -ge 2 ] || usage
      outage_confirmation=$2
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
if [ -n "$outage_confirmation" ] && [ "$outage_confirmation" != "$namespace" ]; then
  echo "refusing --outage-drill: confirmation must repeat the target namespace" >&2
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

plane_get() {
  kube exec deployment/fkst-control-plane -- python3 -c \
    "import sys,urllib.request;sys.stdout.write(urllib.request.urlopen('http://127.0.0.1:8080$1',timeout=10).read().decode())"
}

# Assert that an exposition really PUBLISHES a family, not merely that its name
# appears somewhere in the body.
#
# A substring grep passes on the `# HELP`/`# TYPE` header a renderer emits before
# its loop, so a family whose loop never ran once — an inventory that has served
# no request, a counter block behind a dead code path — looked identical to a
# healthy one. Requiring a TYPE declaration AND at least one sample line makes
# the assertion mean "this deployment can and does report this", which is what
# every caller of it actually wants to know.
assert_family() {
  body=$1
  family=$2
  label=$3
  printf '%s\n' "$body" | awk -v family="$family" '
    $1 == "#" && $2 == "TYPE" && $3 == family { typed = 1; next }
    index($0, family) == 1 {
      rest = substr($0, length(family) + 1)
      # A sample line is `name{labels} value` or `name value`; a longer family
      # name starting with the same prefix is not this family.
      if (rest ~ /^[{ ]/ && NF >= 2) samples++
    }
    END { exit (typed && samples > 0) ? 0 : 1 }
  ' || {
    echo "$label does not publish $family (needs a # TYPE line and at least one sample)" >&2
    exit 1
  }
}

# Sum every `fkst_audit_relay_records{...}` sample in an exposition.
record_total() {
  printf '%s\n' "$1" | awk '/^fkst_audit_relay_records\{/ { total += $2 } END { print total + 0 }'
}

# One bounded sample's value, or the empty string when the series is absent.
sample_value() {
  printf '%s\n' "$1" | awk -v series="$2" '$1 == series { print $2; found = 1 } END { if (!found) print "" }'
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
if [ -z "$storage_class" ] || [ -z "$capacity" ]; then
  echo "the audit outbox claim must report an explicit storage class and capacity" >&2
  exit 1
fi
echo "bound    $capacity on storageClass $storage_class"

echo "== workload =="
kube rollout status deployment/fkst-audit-relay --timeout="$timeout" >/dev/null
replicas=$(read_field deployment.apps fkst-audit-relay '{.spec.replicas}')
strategy=$(read_field deployment.apps fkst-audit-relay '{.spec.strategy.type}')
if [ "$replicas" != "1" ] || [ "$strategy" != "Recreate" ]; then
  echo "the relay must be one Recreate replica (got $replicas/$strategy)" >&2
  exit 1
fi
echo "rollout  1 replica, Recreate"

echo "== shared configuration =="
relay_grace=$(read_field configmap fkst-audit-relay-config '{.data.FKST_AUDIT_INCOMPLETE_GRACE_SECS}')
plane_grace=$(read_field configmap fkst-control-plane-config '{.data.FKST_AUDIT_INCOMPLETE_GRACE_SECS}')
if [ -z "$relay_grace" ] || [ "$relay_grace" != "$plane_grace" ]; then
  echo "FKST_AUDIT_INCOMPLETE_GRACE_SECS disagrees ('$plane_grace' vs '$relay_grace')" >&2
  exit 1
fi
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
# not reach the port at all.
#
# The probe reports a SENTINEL rather than an exit status, because the three ways
# this check can be inconclusive — no wget in the image, no resolver, an exec the
# caller is not allowed to run — all look exactly like "the connection was
# dropped" to a bare `if ! …`. Reporting the cage as enforced because the probe
# never ran is the worst possible outcome for a security control, so anything
# other than an explicit BLOCKED is a failure here.
relay_host="fkst-audit-relay.${namespace}.svc.cluster.local"
probe_script="command -v wget >/dev/null 2>&1 || { echo FKST_PROBE_NO_TOOL; exit 0; }
command -v nslookup >/dev/null 2>&1 || { echo FKST_PROBE_NO_TOOL; exit 0; }
nslookup ${relay_host} >/dev/null 2>&1 || { echo FKST_PROBE_NO_DNS; exit 0; }
if wget -T 5 -q -O - '${relay_url}/ready' >/dev/null 2>&1; then
  echo FKST_PROBE_REACHED
else
  echo FKST_PROBE_BLOCKED
fi"
frontend_probe=$(kube exec deployment/fkst-frontend -- sh -c "$probe_script" 2>/dev/null) || {
  echo "the frontend isolation probe could not run; the NetworkPolicy is unproven" >&2
  exit 1
}
case "$frontend_probe" in
  *FKST_PROBE_BLOCKED*) : ;;
  *FKST_PROBE_REACHED*)
    echo "the relay accepted a connection from the frontend; the NetworkPolicy is not enforced" >&2
    exit 1
    ;;
  *FKST_PROBE_NO_TOOL*)
    echo "the frontend image has no wget/nslookup to probe with; the NetworkPolicy is unproven" >&2
    exit 1
    ;;
  *FKST_PROBE_NO_DNS*)
    echo "the frontend cannot resolve $relay_host; a drop and a typo are indistinguishable" >&2
    exit 1
    ;;
  *)
    echo "the frontend isolation probe returned no verdict; the NetworkPolicy is unproven" >&2
    exit 1
    ;;
esac
echo "blocked  the frontend resolves the relay and is refused at the port"

echo "== bounded telemetry =="
relay_metrics=$(relay_get /metrics)
for family in fkst_audit_relay_up fkst_audit_relay_ingress_ready fkst_audit_relay_records \
  fkst_audit_relay_oldest_record_age_seconds fkst_audit_relay_db_bytes \
  fkst_audit_relay_max_records fkst_audit_relay_dead_letters_total \
  fkst_audit_relay_incomplete_total; do
  assert_family "$relay_metrics" "$family" "the relay exposition"
done
for forbidden in actor_id session_id request_id event_id repo_full_name login; do
  if printf '%s' "$relay_metrics" | grep -Fq "$forbidden"; then
    echo "the relay exposition mentions $forbidden; metrics labels must stay bounded" >&2
    exit 1
  fi
done
# The capacity guard is the denominator of the headroom alert. A zero would make
# that alert silently unevaluable, so it is checked here rather than discovered
# during the incident it was supposed to precede.
relay_max_records=$(sample_value "$relay_metrics" fkst_audit_relay_max_records)
case "$relay_max_records" in
  ''|0)
    echo "the relay publishes no record capacity guard; the headroom alert cannot evaluate" >&2
    exit 1
    ;;
esac
echo "metrics  8 relay families published, guard ${relay_max_records} records, 0 identity tokens"
plane_metrics=$(plane_get /metrics)
# Live inventory must stay answerable independently of PostHog and of the relay:
# these families are PUBLISHED (declared and sampled) whether or not either is
# healthy. The `--outage-drill` below is what proves the independence itself.
for family in fkst_audit_required_rejections_total fkst_operations_activity_queries_total \
  fkst_operations_activity_source_partial_total fkst_operations_sandbox_inventory_requests_total \
  fkst_session_access_registry_generation_state; do
  assert_family "$plane_metrics" "$family" "the control-plane exposition"
done
echo "metrics  5 control-plane audit families published"

if [ -n "$restart_confirmation" ]; then
  echo "== restart and volume persistence =="
  before=$(record_total "$relay_metrics")
  kube rollout restart deployment/fkst-audit-relay >/dev/null
  kube rollout status deployment/fkst-audit-relay --timeout="$timeout" >/dev/null
  after=$(record_total "$(relay_get /metrics)")
  if [ "$after" -lt "$before" ]; then
    echo "records dropped from $before to $after across a restart; the outbox is not persistent" >&2
    exit 1
  fi
  echo "restart  $before records before, $after after (no loss)"
fi

if [ -n "$outage_confirmation" ]; then
  # Three claims this milestone makes are behavioural and cannot be read off a
  # healthy cluster: that a PostHog outage never takes durable ingress down,
  # that live inventory is independent of the relay, and that a relay outage
  # ends in a drain rather than a loss. The drill produces each condition and
  # then asserts it.
  echo "== PostHog outage invariants =="
  # A relay whose delivery target is unreachable is the shape a disposable
  # cluster runs in by default (the reference overlay points at an RFC 2606
  # `.invalid` host). An outbox whose destination is down is doing its job:
  # ingress stays ready, retries stay retryable, and nothing dead-letters.
  outage_ready=$(sample_value "$relay_metrics" fkst_audit_relay_ingress_ready)
  [ "$outage_ready" = "1" ] || {
    echo "durable ingress is $outage_ready while PostHog is the only thing that may be down" >&2
    exit 1
  }
  dead_before=$(printf '%s\n' "$relay_metrics" |
    awk '/^fkst_audit_relay_dead_letters_total\{/ { total += $2 } END { print total + 0 }')
  permanent_before=$(sample_value "$relay_metrics" 'fkst_audit_relay_capture_total{result="permanent"}')
  echo "outbox   ready with ${dead_before} dead letters, ${permanent_before:-0} permanent capture failures"

  echo "== relay outage, inventory independence, and drain =="
  records_before=$(record_total "$relay_metrics")
  kube scale deployment/fkst-audit-relay --replicas=0 >/dev/null
  kube rollout status deployment/fkst-audit-relay --timeout="$timeout" >/dev/null
  # THE independence claim: with no relay at all, the control plane still
  # answers and still publishes live-inventory samples. A grep for the family
  # name on a healthy cluster proves nothing; this does.
  degraded_metrics=$(plane_get /metrics)
  assert_family "$degraded_metrics" fkst_operations_sandbox_inventory_requests_total \
    "the control-plane exposition with the relay scaled to zero"
  assert_family "$degraded_metrics" fkst_operations_activity_source_partial_total \
    "the control-plane exposition with the relay scaled to zero"
  echo "independent  live inventory and the partial-page counter survive a relay outage"
  kube scale deployment/fkst-audit-relay --replicas=1 >/dev/null
  kube rollout status deployment/fkst-audit-relay --timeout="$timeout" >/dev/null
  drained_metrics=$(relay_get /metrics)
  records_after=$(record_total "$drained_metrics")
  if [ "$records_after" -lt "$records_before" ]; then
    echo "records dropped from $records_before to $records_after across the outage" >&2
    exit 1
  fi
  drained_ready=$(sample_value "$drained_metrics" fkst_audit_relay_ingress_ready)
  [ "$drained_ready" = "1" ] || {
    echo "durable ingress did not return after the relay was scaled back up" >&2
    exit 1
  }
  dead_after=$(printf '%s\n' "$drained_metrics" |
    awk '/^fkst_audit_relay_dead_letters_total\{/ { total += $2 } END { print total + 0 }')
  if [ "$dead_after" -gt "$dead_before" ]; then
    echo "the outage dead-lettered records ($dead_before -> $dead_after); it must only retry" >&2
    exit 1
  fi
  echo "drain    ingress ready again, $records_after records, no new dead letters"
fi

echo "audit relay contract verified in $namespace on context $context"
