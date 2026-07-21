#!/bin/sh
set -eu

usage() {
  echo "usage: $0 --context CONTEXT [--overlay PATH] [--migration-overlay PATH] [--monitoring-overlay PATH]" >&2
  exit 2
}

context=""
script_dir=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
overlay="$script_dir/overlays/local"
migration_overlay="$script_dir/overlays/local-migration"
monitoring_overlay="$script_dir/monitoring"

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
    *) usage ;;
  esac
done

[ -n "$context" ] || usage
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

first=$(mktemp "${TMPDIR:-/tmp}/fkst-render.XXXXXX")
second=$(mktemp "${TMPDIR:-/tmp}/fkst-render.XXXXXX")
migration_first=$(mktemp "${TMPDIR:-/tmp}/fkst-migration-render.XXXXXX")
migration_second=$(mktemp "${TMPDIR:-/tmp}/fkst-migration-render.XXXXXX")
monitoring_first=$(mktemp "${TMPDIR:-/tmp}/fkst-monitoring-render.XXXXXX")
monitoring_second=$(mktemp "${TMPDIR:-/tmp}/fkst-monitoring-render.XXXXXX")
base_first=$(mktemp "${TMPDIR:-/tmp}/fkst-base-render.XXXXXX")
base_second=$(mktemp "${TMPDIR:-/tmp}/fkst-base-render.XXXXXX")
trap 'rm -f "$first" "$second" "$migration_first" "$migration_second" "$monitoring_first" "$monitoring_second" "$base_first" "$base_second"' EXIT HUP INT TERM

kubectl --context "$context" kustomize "$overlay" >"$first"
kubectl --context "$context" kustomize "$overlay" >"$second"
cmp "$first" "$second" >/dev/null
ruby "$script_dir/validate-render.rb" "$first" steady

kubectl --context "$context" kustomize "$migration_overlay" >"$migration_first"
kubectl --context "$context" kustomize "$migration_overlay" >"$migration_second"
cmp "$migration_first" "$migration_second" >/dev/null
ruby "$script_dir/validate-render.rb" "$migration_first" migration

kubectl --context "$context" kustomize "$monitoring_overlay" >"$monitoring_first"
kubectl --context "$context" kustomize "$monitoring_overlay" >"$monitoring_second"
cmp "$monitoring_first" "$monitoring_second" >/dev/null
kubectl --context "$context" kustomize "$script_dir/base" >"$base_first"
kubectl --context "$context" kustomize "$script_dir/base" >"$base_second"
cmp "$base_first" "$base_second" >/dev/null
ruby "$script_dir/validate-monitoring.rb" "$monitoring_first" "$base_first" "$first"

sh -n "$script_dir/migrate-environment-store.sh"
sh -n "$script_dir/restore-namespace.sh"
sh -n "$script_dir/verify-namespace.sh"
sh -n "$script_dir/verify-envstore-rbac.sh"
sh -n "$script_dir/run-disaster-drill.sh"
sh -n "$script_dir/tests/disaster-drill-test.sh"
ruby -c "$script_dir/render-recovery-evidence.rb" >/dev/null
ruby -c "$script_dir/validate-monitoring.rb" >/dev/null
if command -v shellcheck >/dev/null 2>&1; then
  shellcheck "$script_dir/migrate-environment-store.sh" \
    "$script_dir/restore-namespace.sh" \
    "$script_dir/verify-namespace.sh" \
    "$script_dir/verify-envstore-rbac.sh" \
    "$script_dir/run-disaster-drill.sh" \
    "$script_dir/tests/disaster-drill-test.sh" \
    "$script_dir/validate-manifests.sh"
fi

"$script_dir/tests/disaster-drill-test.sh"

if command -v kubeconform >/dev/null 2>&1; then
  kubeconform -strict -summary -ignore-missing-schemas - <"$first"
  kubeconform -strict -summary -ignore-missing-schemas - <"$migration_first"
  kubeconform -strict -summary -ignore-missing-schemas - <"$monitoring_first"
  kubeconform -strict -summary -ignore-missing-schemas - <"$base_first"
else
  echo "kubeconform not found; structural policy validation passed (schema validation skipped)" >&2
fi

echo "canonical Kubernetes manifests render deterministically"
