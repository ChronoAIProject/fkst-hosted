#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
LOCAL_PACKAGES_ROOT="$ROOT/.fkst/local-packages"
EXTERNAL_PACKAGES_ROOT="$ROOT/.fkst/packages"

# shellcheck source=scripts/composed_manifest.sh
. "$ROOT/scripts/composed_manifest.sh"

package_root_for_name() {
  local name="$1"
  if [ -d "$LOCAL_PACKAGES_ROOT/$name" ]; then
    printf '%s\n' "$LOCAL_PACKAGES_ROOT/$name"
    return 0
  fi
  if [ -d "$EXTERNAL_PACKAGES_ROOT/$name" ]; then
    printf '%s\n' "$EXTERNAL_PACKAGES_ROOT/$name"
    return 0
  fi
  return 1
}

csv_contains() {
  local csv="$1" needle="$2" item
  IFS=',' read -r -a items <<< "$csv"
  for item in "${items[@]}"; do
    [ "$item" = "$needle" ] && return 0
  done
  return 1
}

collect_composed_package() {
  local name="$1" excluded_csv="$2" pkg dep deps rc
  if csv_contains "$excluded_csv" "$name"; then
    return 0
  fi
  pkg="$(package_root_for_name "$name")" || { echo "error: composed package dependency not found: $name" >&2; return 1; }
  [ -d "$pkg" ] || { echo "error: composed package dependency not found: $name" >&2; return 1; }
  case " ${COMPOSED_SEEN[*]-} " in
    *" $name "*) return 0 ;;
  esac
  COMPOSED_SEEN+=("$name")
  set +e; deps="$(composition_siblings_of "$pkg")"; rc=$?; set -e
  case "$rc" in
    0)
      while IFS= read -r dep || [ -n "$dep" ]; do
        [ -n "$dep" ] || continue
        collect_composed_package "$dep" "$excluded_csv" || return 1
      done <<< "$deps"
      ;;
    1) return 0 ;;
    *) echo "error: failed to read package composition for $pkg" >&2; return 1 ;;
  esac
}

run_topology() {
  local topology="$1" excluded_csv="$2" pkg name args project_root rc
  COMPOSED_SEEN=()
  for pkg in "$LOCAL_PACKAGES_ROOT"/*/ "$EXTERNAL_PACKAGES_ROOT"/*/; do
    [ -d "$pkg" ] || continue
    name="$(basename "$pkg")"
    if csv_contains "$excluded_csv" "$name"; then
      continue
    fi
    rc=0; is_composed "$pkg" || rc=$?
    case "$rc" in
      0) ;;
      1) continue ;;
      *) echo "error: failed to read package composition for $pkg" >&2; return 1 ;;
    esac
    collect_composed_package "$name" "$excluded_csv" || return 1
  done
  if [ "${#COMPOSED_SEEN[@]}" -eq 0 ]; then
    echo "no composed packages matched for topology: $topology"
    return 0
  fi

  python3 -B "$ROOT/scripts/check_repo_intake_routing.py" --assert-topology "$ROOT" --packages "${COMPOSED_SEEN[@]}"

  args=()
  project_root="$(package_root_for_name "${COMPOSED_SEEN[0]}")" || return 1
  for name in "${COMPOSED_SEEN[@]}"; do
    pkg="$(package_root_for_name "$name")" || return 1
    args+=(--package-root "$pkg")
  done
  for pkg in "$LOCAL_PACKAGES_ROOT"/*/ "$EXTERNAL_PACKAGES_ROOT"/*/; do
    [ -d "$pkg" ] || continue
    name="$(basename "$pkg")"
    if csv_contains "$excluded_csv" "$name"; then
      continue
    fi
    case " ${COMPOSED_SEEN[*]} " in
      *" $name "*) continue ;;
    esac
    args+=(--package-root "${pkg%/}")
  done
  echo "=== composed conformance: $topology ==="
  run_quiet_pass "$BIN" conformance --project-root "$project_root" "${args[@]}"
}

cmd_test_composed() {
  local rows topology excluded_csv any=0
  ensure_package_view
  rows="$(python3 -B "$ROOT/scripts/check_repo_intake_routing.py" --topology-rows "$ROOT")"
  while IFS=$'\t' read -r topology excluded_csv || [ -n "$topology" ]; do
    [ -n "$topology" ] || continue
    any=1
    run_topology "$topology" "$excluded_csv" || return 1
  done <<< "$rows"
  if [ "$any" -eq 0 ]; then
    echo "no composed package topologies matched"
  fi
}
