#!/bin/sh
set -eu

# Renders every checked-in Kustomization twice, proves the renders are identical,
# and puts them through the structural/security policy validators plus the shell
# and Ruby linters.
#
# It contacts NO cluster: `kubectl kustomize` is a local render, and every other
# step reads files. `--context` is therefore OPTIONAL and exists only so an
# operator following a runbook keeps naming the context they are working against
# — the value is passed through to `kubectl` and changes nothing. Omitting it is
# what makes this runnable on a machine (or a CI runner) with no kubeconfig at
# all, which is the only way the checks below can guard a change before it
# reaches a cluster.

usage() {
  echo "usage: $0 [--context CONTEXT] [--overlay PATH] [--migration-overlay PATH] [--monitoring-overlay PATH] [--audit-relay-overlay PATH] [--required-audit-overlay PATH]" >&2
  exit 2
}

context=""
script_dir=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
overlay="$script_dir/overlays/local"
migration_overlay="$script_dir/overlays/local-migration"
monitoring_overlay="$script_dir/monitoring"
audit_relay_overlay="$script_dir/audit-relay"
required_audit_overlay="$script_dir/overlays/required-audit"

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
    --migration-overlay)
      [ "$#" -ge 2 ] || usage
      migration_overlay=$2
      shift 2
      ;;
    --monitoring-overlay)
      [ "$#" -ge 2 ] || usage
      monitoring_overlay=$2
      shift 2
      ;;
    --audit-relay-overlay)
      [ "$#" -ge 2 ] || usage
      audit_relay_overlay=$2
      shift 2
      ;;
    --required-audit-overlay)
      [ "$#" -ge 2 ] || usage
      required_audit_overlay=$2
      shift 2
      ;;
    *) usage ;;
  esac
done

[ -f "$overlay/kustomization.yaml" ] || {
  echo "overlay has no kustomization.yaml: $overlay" >&2
  exit 1
}
[ -f "$migration_overlay/kustomization.yaml" ] || {
  echo "migration overlay has no kustomization.yaml: $migration_overlay" >&2
  exit 1
}
[ -f "$monitoring_overlay/kustomization.yaml" ] || {
  echo "monitoring overlay has no kustomization.yaml: $monitoring_overlay" >&2
  exit 1
}
[ -f "$audit_relay_overlay/kustomization.yaml" ] || {
  echo "audit-relay overlay has no kustomization.yaml: $audit_relay_overlay" >&2
  exit 1
}
[ -f "$required_audit_overlay/kustomization.yaml" ] || {
  echo "required-audit overlay has no kustomization.yaml: $required_audit_overlay" >&2
  exit 1
}

# One render entry point, so the context stays optional in exactly one place.
render() {
  if [ -n "$context" ]; then
    kubectl --context "$context" kustomize "$@"
  else
    kubectl kustomize "$@"
  fi
}

first=$(mktemp "${TMPDIR:-/tmp}/fkst-render.XXXXXX")
second=$(mktemp "${TMPDIR:-/tmp}/fkst-render.XXXXXX")
migration_first=$(mktemp "${TMPDIR:-/tmp}/fkst-migration-render.XXXXXX")
migration_second=$(mktemp "${TMPDIR:-/tmp}/fkst-migration-render.XXXXXX")
monitoring_first=$(mktemp "${TMPDIR:-/tmp}/fkst-monitoring-render.XXXXXX")
monitoring_second=$(mktemp "${TMPDIR:-/tmp}/fkst-monitoring-render.XXXXXX")
base_first=$(mktemp "${TMPDIR:-/tmp}/fkst-base-render.XXXXXX")
base_second=$(mktemp "${TMPDIR:-/tmp}/fkst-base-render.XXXXXX")
relay_first=$(mktemp "${TMPDIR:-/tmp}/fkst-relay-render.XXXXXX")
relay_second=$(mktemp "${TMPDIR:-/tmp}/fkst-relay-render.XXXXXX")
required_first=$(mktemp "${TMPDIR:-/tmp}/fkst-required-render.XXXXXX")
required_second=$(mktemp "${TMPDIR:-/tmp}/fkst-required-render.XXXXXX")
secrets_first=$(mktemp "${TMPDIR:-/tmp}/fkst-secrets-render.XXXXXX")
secrets_second=$(mktemp "${TMPDIR:-/tmp}/fkst-secrets-render.XXXXXX")
trap 'rm -f "$first" "$second" "$migration_first" "$migration_second" "$monitoring_first" "$monitoring_second" "$base_first" "$base_second" "$relay_first" "$relay_second" "$required_first" "$required_second" "$secrets_first" "$secrets_second"' EXIT HUP INT TERM

render "$overlay" >"$first"
render "$overlay" >"$second"
cmp "$first" "$second" >/dev/null
ruby "$script_dir/validate-render.rb" "$first" steady

render "$migration_overlay" >"$migration_first"
render "$migration_overlay" >"$migration_second"
cmp "$migration_first" "$migration_second" >/dev/null
ruby "$script_dir/validate-render.rb" "$migration_first" migration

render "$monitoring_overlay" >"$monitoring_first"
render "$monitoring_overlay" >"$monitoring_second"
cmp "$monitoring_first" "$monitoring_second" >/dev/null
render "$script_dir/base" >"$base_first"
render "$script_dir/base" >"$base_second"
cmp "$base_first" "$base_second" >/dev/null
ruby "$script_dir/validate-monitoring.rb" "$monitoring_first" "$base_first" "$first"

# The audit relay and the composed required-delivery shape. The relay render is
# checked twice for determinism like the others, and the composed overlay is put
# through BOTH validators: the steady namespace policy (it is a superset of the
# local overlay) and the relay-specific policy.
render "$audit_relay_overlay" >"$relay_first"
render "$audit_relay_overlay" >"$relay_second"
cmp "$relay_first" "$relay_second" >/dev/null
render "$required_audit_overlay" >"$required_first"
render "$required_audit_overlay" >"$required_second"
cmp "$required_first" "$required_second" >/dev/null
ruby "$script_dir/validate-render.rb" "$required_first" steady
ruby "$script_dir/validate-audit-relay.rb" "$relay_first" "$required_first" "$base_first"

# The provider-neutral credential bindings, rendered on their own: they are
# applied before workloads during a restore, so they must stand alone.
render "$script_dir/external-secrets" >"$secrets_first"
render "$script_dir/external-secrets" >"$secrets_second"
cmp "$secrets_first" "$secrets_second" >/dev/null

sh -n "$script_dir/migrate-environment-store.sh"
sh -n "$script_dir/restore-namespace.sh"
sh -n "$script_dir/verify-namespace.sh"
sh -n "$script_dir/verify-envstore-rbac.sh"
sh -n "$script_dir/run-disaster-drill.sh"
sh -n "$script_dir/verify-audit-relay.sh"
sh -n "$script_dir/tests/disaster-drill-test.sh"
sh -n "$script_dir/tests/audit-relay-verify-test.sh"
ruby -c "$script_dir/render-recovery-evidence.rb" >/dev/null
ruby -c "$script_dir/validate-monitoring.rb" >/dev/null
ruby -c "$script_dir/validate-audit-relay.rb" >/dev/null
if command -v shellcheck >/dev/null 2>&1; then
  shellcheck "$script_dir/migrate-environment-store.sh" \
    "$script_dir/restore-namespace.sh" \
    "$script_dir/verify-namespace.sh" \
    "$script_dir/verify-envstore-rbac.sh" \
    "$script_dir/verify-audit-relay.sh" \
    "$script_dir/run-disaster-drill.sh" \
    "$script_dir/tests/disaster-drill-test.sh" \
    "$script_dir/tests/audit-relay-verify-test.sh" \
    "$script_dir/validate-manifests.sh"
fi

"$script_dir/tests/disaster-drill-test.sh"
"$script_dir/tests/audit-relay-verify-test.sh"

if command -v kubeconform >/dev/null 2>&1; then
  kubeconform -strict -summary -ignore-missing-schemas - <"$first"
  kubeconform -strict -summary -ignore-missing-schemas - <"$migration_first"
  kubeconform -strict -summary -ignore-missing-schemas - <"$monitoring_first"
  kubeconform -strict -summary -ignore-missing-schemas - <"$base_first"
  kubeconform -strict -summary -ignore-missing-schemas - <"$relay_first"
  kubeconform -strict -summary -ignore-missing-schemas - <"$required_first"
  kubeconform -strict -summary -ignore-missing-schemas - <"$secrets_first"
else
  echo "kubeconform not found; structural policy validation passed (schema validation skipped)" >&2
fi

echo "canonical Kubernetes manifests render deterministically"
