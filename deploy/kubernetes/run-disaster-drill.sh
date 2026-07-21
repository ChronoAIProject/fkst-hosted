#!/bin/sh
set -eu

usage() {
  echo "usage: $0 --context CONTEXT --target-namespace chronoai-fkst --confirm-delete chronoai-fkst --durable-namespace NAMESPACE --repository OWNER/REPO --sentinel-user-id ID --sentinel-name NAME --evidence-dir PATH [--timeout-seconds SECONDS]" >&2
  exit 2
}

context=""
target_namespace=""
confirmation=""
durable_namespace=""
repository=""
sentinel_user_id=""
sentinel_name=""
evidence_dir=""
timeout_seconds="1800"
script_dir=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
overlay="$script_dir/overlays/local"
restore_script="$script_dir/restore-namespace.sh"
renderer="$script_dir/render-recovery-evidence.rb"

while [ "$#" -gt 0 ]; do
  case "$1" in
    --context)
      [ "$#" -ge 2 ] || usage
      context=$2
      shift 2
      ;;
    --target-namespace)
      [ "$#" -ge 2 ] || usage
      target_namespace=$2
      shift 2
      ;;
    --confirm-delete)
      [ "$#" -ge 2 ] || usage
      confirmation=$2
      shift 2
      ;;
    --durable-namespace)
      [ "$#" -ge 2 ] || usage
      durable_namespace=$2
      shift 2
      ;;
    --repository)
      [ "$#" -ge 2 ] || usage
      repository=$2
      shift 2
      ;;
    --sentinel-user-id)
      [ "$#" -ge 2 ] || usage
      sentinel_user_id=$2
      shift 2
      ;;
    --sentinel-name)
      [ "$#" -ge 2 ] || usage
      sentinel_name=$2
      shift 2
      ;;
    --evidence-dir)
      [ "$#" -ge 2 ] || usage
      evidence_dir=$2
      shift 2
      ;;
    --timeout-seconds)
      [ "$#" -ge 2 ] || usage
      timeout_seconds=$2
      shift 2
      ;;
    *) usage ;;
  esac
done

for required in "$context" "$target_namespace" "$confirmation" "$durable_namespace" \
  "$repository" "$sentinel_user_id" "$sentinel_name" "$evidence_dir"; do
  [ -n "$required" ] || usage
done

started_at=$(date -u +%Y-%m-%dT%H:%M:%SZ)
result="failed"
failure_phase="preflight"
context_class=""
rto_seconds=""
repository_sha256=""
pre_session_count=""
post_session_count=""
pre_session_set_sha256=""
post_session_set_sha256=""
environment_content_hash=""
environment_secret_key_count=""
environment_secret_keys_sha256=""
lease_transitions_before=""
lease_transitions_after=""
pre_sessions=""
post_sessions=""
inventory_rows=""
inventory_ids=""
environment_keys=""
post_environment_keys=""
evidence_enabled="false"

cleanup() {
  for path in "$pre_sessions" "$post_sessions" "$inventory_rows" "$inventory_ids" \
    "$environment_keys" "$post_environment_keys"; do
    [ -z "$path" ] || [ ! -e "$path" ] || rm -f -- "$path"
  done
}

emit_evidence() {
  status=$?
  trap - EXIT HUP INT TERM
  completed_at=$(date -u +%Y-%m-%dT%H:%M:%SZ)
  if [ "$evidence_enabled" = "true" ]; then
    if ! ruby "$renderer" \
      --output-dir "$evidence_dir" \
      --result "$result" \
      --failure-phase "$failure_phase" \
      --started-at "$started_at" \
      --completed-at "$completed_at" \
      --context-class "$context_class" \
      --target-namespace "$target_namespace" \
      --rto-seconds "$rto_seconds" \
      --repository-sha256 "$repository_sha256" \
      --pre-session-count "$pre_session_count" \
      --post-session-count "$post_session_count" \
      --pre-session-set-sha256 "$pre_session_set_sha256" \
      --post-session-set-sha256 "$post_session_set_sha256" \
      --environment-content-hash "$environment_content_hash" \
      --environment-secret-key-count "$environment_secret_key_count" \
      --environment-secret-keys-sha256 "$environment_secret_keys_sha256" \
      --lease-transitions-before "$lease_transitions_before" \
      --lease-transitions-after "$lease_transitions_after"; then
      echo "failed to render bounded recovery evidence" >&2
      [ "$status" -ne 0 ] || status=1
    fi
  fi
  cleanup
  exit "$status"
}

trap emit_evidence EXIT
trap 'exit 130' HUP INT TERM

fail() {
  echo "$1" >&2
  exit 1
}

hash_stdin() {
  ruby -rdigest -e 'print Digest::SHA256.hexdigest(STDIN.read)'
}

hash_file() {
  ruby -rdigest -e 'print Digest::SHA256.file(ARGV.fetch(0)).hexdigest' "$1"
}

case "$timeout_seconds" in
  ''|*[!0-9]*) fail "timeout must be an integer number of seconds" ;;
esac
[ "$timeout_seconds" -ge 1 ] && [ "$timeout_seconds" -le 3600 ] || \
  fail "timeout must be between 1 and 3600 seconds"

case "$context" in
  kind-*) ;;
  *) fail "refusing disaster drill: context is not an explicit kind context" ;;
esac
context_lower=$(printf '%s' "$context" | tr '[:upper:]' '[:lower:]')
case "$context_lower" in
  *prod*|*live*|*staging*|*main*)
    fail "refusing disaster drill: context name is production-like"
    ;;
esac
context_class="kind_disposable"

[ "$target_namespace" = "chronoai-fkst" ] || \
  fail "refusing disaster drill: only the canonical chronoai-fkst target is supported"
[ "$confirmation" = "$target_namespace" ] || \
  fail "refusing disaster drill: deletion confirmation does not exactly match the target"
[ "$durable_namespace" != "$target_namespace" ] || \
  fail "refusing disaster drill: durable source must differ from the disposable target"
printf '%s\n' "$durable_namespace" | grep -Eq '^[a-z0-9]([-a-z0-9]*[a-z0-9])?$' || \
  fail "refusing disaster drill: durable namespace is not a DNS label"

case "$repository" in
  */*) ;;
  *) fail "repository must be an exact OWNER/REPO pair" ;;
esac
repo_owner=${repository%%/*}
repo_name=${repository#*/}
[ "$repository" = "$repo_owner/$repo_name" ] && [ -n "$repo_owner" ] && [ -n "$repo_name" ] || \
  fail "repository must contain exactly one slash"
for segment in "$repo_owner" "$repo_name"; do
  printf '%s\n' "$segment" | grep -Eq '^[A-Za-z0-9_.-]+$' || \
    fail "repository segments contain unsupported characters"
  [ "${#segment}" -le 63 ] || fail "repository segments must fit Kubernetes correlation labels"
done
repository_sha256=$(printf '%s' "$repository" | hash_stdin)

printf '%s\n' "$sentinel_user_id" | grep -Eq '^[0-9]+$' || \
  fail "sentinel user id must be numeric"
printf '%s\n' "$sentinel_name" | grep -Eq '^[a-z0-9]([-a-z0-9]*[a-z0-9])?$' || \
  fail "sentinel name must be normalized as a DNS label"
[ "${#sentinel_name}" -le 63 ] || fail "sentinel name is too long"
sentinel_secret="fkst-env-${sentinel_user_id}-${sentinel_name}"
[ "${#sentinel_secret}" -le 253 ] || fail "sentinel Secret name is too long"

[ -f "$overlay/kustomization.yaml" ] || fail "canonical local overlay is missing"
[ -x "$restore_script" ] || fail "canonical restore runner is missing or not executable"
[ -f "$renderer" ] || fail "redacted evidence renderer is missing"
command -v kubectl >/dev/null 2>&1 || fail "kubectl is required"
command -v ruby >/dev/null 2>&1 || fail "ruby is required"
command -v cmp >/dev/null 2>&1 || fail "cmp is required"

mkdir -p -- "$evidence_dir"
evidence_enabled="true"
pre_sessions=$(mktemp "${TMPDIR:-/tmp}/fkst-drill-pre-sessions.XXXXXX")
post_sessions=$(mktemp "${TMPDIR:-/tmp}/fkst-drill-post-sessions.XXXXXX")
inventory_rows=$(mktemp "${TMPDIR:-/tmp}/fkst-drill-runtime-rows.XXXXXX")
inventory_ids=$(mktemp "${TMPDIR:-/tmp}/fkst-drill-runtime-ids.XXXXXX")
environment_keys=$(mktemp "${TMPDIR:-/tmp}/fkst-drill-environment-keys.XXXXXX")
post_environment_keys=$(mktemp "${TMPDIR:-/tmp}/fkst-drill-post-environment-keys.XXXXXX")

kubectl --context "$context" config view --minify --output name >/dev/null

assert_safety_labels() {
  disposable=$(kubectl --context "$context" get namespace "$target_namespace" \
    --output jsonpath='{.metadata.labels.fkst\.chronoai\.io/disposable}')
  [ "$disposable" = "true" ] || \
    fail "refusing disaster drill: target namespace is not explicitly disposable"
  boundary=$(kubectl --context "$context" get namespace "$durable_namespace" \
    --output jsonpath='{.metadata.labels.fkst\.chronoai\.io/durability-boundary}')
  [ "$boundary" = "external" ] || \
    fail "refusing disaster drill: durable namespace lacks the external boundary label"
}

inventory_live_sessions() {
  destination=$1
  selector="fkst-managed=true,fkst-owner=${repo_owner},fkst-repo=${repo_name}"
  template='{{range .items}}{{index .metadata.labels "fkst-session-id"}}{{"\t"}}{{.status.phase}}{{"\t"}}{{if .metadata.deletionTimestamp}}deleting{{else}}active{{end}}{{"\t"}}{{range .status.containerStatuses}}{{.ready}}{{","}}{{end}}{{"\n"}}{{end}}'
  kubectl --context "$context" --namespace "$target_namespace" get pods \
    --selector "$selector" --output "go-template=$template" >"$inventory_rows"
  : >"$inventory_ids"
  tab=$(printf '\t')
  while IFS="$tab" read -r session_id phase lifecycle readiness; do
    [ "$phase" = "Running" ] || continue
    [ "$lifecycle" = "active" ] || continue
    [ -n "$readiness" ] || continue
    case "$readiness" in *false*) continue ;; esac
    printf '%s\n' "$session_id" | grep -Eq '^[A-Za-z0-9._-]{1,128}$' || \
      fail "runtime inventory contains an invalid deterministic session id"
    printf '%s\n' "$session_id" >>"$inventory_ids"
  done <"$inventory_rows"
  duplicate=$(LC_ALL=C sort "$inventory_ids" | uniq -d | sed -n '1p')
  [ -z "$duplicate" ] || fail "runtime inventory contains a duplicate deterministic session id"
  LC_ALL=C sort "$inventory_ids" >"$destination"
}

read_environment_keys() {
  destination=$1
  keys_json=$(kubectl --context "$context" --namespace "$durable_namespace" get \
    secret "$sentinel_secret" \
    --output go-template='{{index .metadata.annotations "fkst.chrono-ai.fun/env-secret-keys"}}')
  printf '%s' "$keys_json" | ruby -rjson -e '
    keys = JSON.parse(STDIN.read)
    abort "environment secret-key annotation must be an array" unless keys.is_a?(Array)
    abort "environment secret-key annotation contains an invalid key" unless keys.all? { |key| key.is_a?(String) && key.match?(/\A[A-Za-z_][A-Za-z0-9_]*\z/) }
    abort "environment secret-key annotation contains duplicates" unless keys.uniq.length == keys.length
    puts keys.sort
  ' >"$destination"
}

assert_safety_labels

failure_phase="runtime_inventory"
inventory_live_sessions "$pre_sessions"
pre_session_count=$(wc -l <"$pre_sessions" | tr -d ' ')
[ "$pre_session_count" -gt 0 ] || \
  fail "refusing disaster drill: repository has no prepared live runtime"
pre_session_set_sha256=$(hash_file "$pre_sessions")

failure_phase="environment_inventory"
environment_content_hash=$(kubectl --context "$context" --namespace "$durable_namespace" get \
  secret "$sentinel_secret" \
  --output go-template='{{index .metadata.annotations "fkst.chrono-ai.fun/content-hash"}}')
printf '%s\n' "$environment_content_hash" | grep -Eq '^[0-9a-f]{64}$' || \
  fail "durable environment sentinel has no valid content hash"
# shellcheck disable=SC2016
data_keys=$(kubectl --context "$context" --namespace "$durable_namespace" get \
  secret "$sentinel_secret" \
  --output go-template='{{range $key, $_ := .data}}{{$key}}{{"\n"}}{{end}}' | LC_ALL=C sort)
[ "$data_keys" = "ciphertext
nonce" ] || fail "durable environment sentinel must expose only encrypted data keys"
read_environment_keys "$environment_keys"
environment_secret_key_count=$(wc -l <"$environment_keys" | tr -d ' ')
environment_secret_keys_sha256=$(hash_file "$environment_keys")

lease_transitions_before=$(kubectl --context "$context" --namespace "$target_namespace" get \
  lease.coordination.k8s.io fkst-control-plane-reconciler \
  --output jsonpath='{.spec.leaseTransitions}')
case "$lease_transitions_before" in
  ''|*[!0-9]*) fail "pre-drill Lease transition count is missing or invalid" ;;
esac

failure_phase="restore_preflight"
"$restore_script" --context "$context" --overlay "$overlay" \
  --timeout "${timeout_seconds}s" --durable-namespace "$durable_namespace" --preflight-only

# Repeat every mutable safety gate immediately before the single destructive call.
assert_safety_labels
[ "$confirmation" = "$target_namespace" ] || \
  fail "refusing disaster drill: deletion confirmation changed"
failure_phase="runtime_inventory"
inventory_live_sessions "$post_sessions"
cmp "$pre_sessions" "$post_sessions" >/dev/null || \
  fail "refusing disaster drill: prepared runtime set changed during preflight"
failure_phase="environment_inventory"
pre_delete_environment_hash=$(kubectl --context "$context" \
  --namespace "$durable_namespace" get secret "$sentinel_secret" \
  --output go-template='{{index .metadata.annotations "fkst.chrono-ai.fun/content-hash"}}')
[ "$pre_delete_environment_hash" = "$environment_content_hash" ] || \
  fail "refusing disaster drill: environment sentinel changed during preflight"
read_environment_keys "$post_environment_keys"
cmp "$environment_keys" "$post_environment_keys" >/dev/null || \
  fail "refusing disaster drill: environment key inventory changed during preflight"

failure_phase="namespace_delete"
rto_started_epoch=$(date +%s)
echo "deleting the confirmed disposable namespace"
kubectl --context "$context" delete namespace "$target_namespace" \
  --wait=true --timeout="${timeout_seconds}s"

failure_phase="namespace_restore"
environment_keys_csv=$(paste -sd, "$environment_keys")
"$restore_script" --context "$context" --overlay "$overlay" \
  --timeout "${timeout_seconds}s" --durable-namespace "$durable_namespace" \
  --sentinel-user-id "$sentinel_user_id" --sentinel-name "$sentinel_name" \
  --sentinel-content-hash "$environment_content_hash" \
  --sentinel-secret-keys "$environment_keys_csv"

failure_phase="runtime_reconstruction"
deadline=$((rto_started_epoch + timeout_seconds))
while :; do
  inventory_live_sessions "$post_sessions"
  if cmp "$pre_sessions" "$post_sessions" >/dev/null; then
    break
  fi
  [ "$(date +%s)" -lt "$deadline" ] || \
    fail "deterministic session set did not recover before the deadline"
  sleep 5
done
post_session_count=$(wc -l <"$post_sessions" | tr -d ' ')
post_session_set_sha256=$(hash_file "$post_sessions")

failure_phase="post_verify"
post_environment_hash=$(kubectl --context "$context" --namespace "$durable_namespace" get \
  secret "$sentinel_secret" \
  --output go-template='{{index .metadata.annotations "fkst.chrono-ai.fun/content-hash"}}')
[ "$post_environment_hash" = "$environment_content_hash" ] || \
  fail "durable environment content hash changed after restoration"
read_environment_keys "$post_environment_keys"
cmp "$environment_keys" "$post_environment_keys" >/dev/null || \
  fail "durable environment secret-key inventory changed after restoration"

lease_transitions_after=$(kubectl --context "$context" --namespace "$target_namespace" get \
  lease.coordination.k8s.io fkst-control-plane-reconciler \
  --output jsonpath='{.spec.leaseTransitions}')
case "$lease_transitions_after" in
  ''|*[!0-9]*) fail "post-drill Lease transition count is missing or invalid" ;;
esac

rto_seconds=$(($(date +%s) - rto_started_epoch))
[ "$rto_seconds" -le "$timeout_seconds" ] || \
  fail "namespace recovery exceeded the configured RTO deadline"
result="passed"
failure_phase="none"
echo "disposable namespace recovery drill passed in ${rto_seconds}s"
