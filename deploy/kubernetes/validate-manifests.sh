#!/bin/sh
set -eu

usage() {
  echo "usage: $0 --context CONTEXT [--overlay PATH] [--migration-overlay PATH]" >&2
  exit 2
}

context=""
script_dir=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
overlay="$script_dir/overlays/local"
migration_overlay="$script_dir/overlays/local-migration"

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

first=$(mktemp "${TMPDIR:-/tmp}/fkst-render.XXXXXX")
second=$(mktemp "${TMPDIR:-/tmp}/fkst-render.XXXXXX")
migration_first=$(mktemp "${TMPDIR:-/tmp}/fkst-migration-render.XXXXXX")
migration_second=$(mktemp "${TMPDIR:-/tmp}/fkst-migration-render.XXXXXX")
trap 'rm -f "$first" "$second" "$migration_first" "$migration_second"' EXIT HUP INT TERM

kubectl --context "$context" kustomize "$overlay" >"$first"
kubectl --context "$context" kustomize "$overlay" >"$second"
cmp "$first" "$second" >/dev/null
ruby "$script_dir/validate-render.rb" "$first" steady

kubectl --context "$context" kustomize "$migration_overlay" >"$migration_first"
kubectl --context "$context" kustomize "$migration_overlay" >"$migration_second"
cmp "$migration_first" "$migration_second" >/dev/null
ruby "$script_dir/validate-render.rb" "$migration_first" migration

sh -n "$script_dir/migrate-environment-store.sh"
sh -n "$script_dir/restore-namespace.sh"
sh -n "$script_dir/verify-namespace.sh"
sh -n "$script_dir/verify-envstore-rbac.sh"
if command -v shellcheck >/dev/null 2>&1; then
  shellcheck "$script_dir/migrate-environment-store.sh" \
    "$script_dir/restore-namespace.sh" \
    "$script_dir/verify-namespace.sh" \
    "$script_dir/verify-envstore-rbac.sh" \
    "$script_dir/validate-manifests.sh"
fi

if command -v kubeconform >/dev/null 2>&1; then
  kubeconform -strict -summary -ignore-missing-schemas "$first"
  kubeconform -strict -summary -ignore-missing-schemas "$migration_first"
else
  echo "kubeconform not found; structural policy validation passed (schema validation skipped)" >&2
fi

echo "canonical Kubernetes manifests render deterministically"
