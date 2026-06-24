#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
LOCAL_PACKAGES_ROOT="$ROOT/.fkst/local-packages"
EXTERNAL_PACKAGES_ROOT="$ROOT/.fkst/packages"

# shellcheck source=scripts/composed_manifest.sh
. "$ROOT/scripts/composed_manifest.sh"

usage() {
  echo "usage: scripts/composed_test_graph_roots.sh <normal|graph> <package>" >&2
}

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

shell_quote() {
  printf "'%s'" "$(printf '%s' "$1" | sed "s/'/'\\\\''/g")"
}

write_roots() {
  local project_root="$1"; shift
  {
    printf 'test_project_root='
    shell_quote "$project_root"
    printf '\n'
    printf 'test_pkg_args=('
    while [ "$#" -gt 0 ]; do
      printf -- '--package-root '
      shell_quote "$1"
      [ "$#" -gt 1 ] && printf ' '
      shift
    done
    printf ')\n'
  }
}

collect_package() {
  local name="$1" pkg dep deps rc
  pkg="$(package_root_for_name "$name")" || {
    echo "error: composed package dependency not found: $name" >&2
    return 1
  }
  case " ${seen[*]-} " in
    *" $name "*) return 0 ;;
  esac
  seen+=("$name")
  set +e
  deps="$(composition_siblings_of "$pkg")"
  rc=$?
  set -e
  case "$rc" in
    0)
      while IFS= read -r dep || [ -n "$dep" ]; do
        [ -n "$dep" ] || continue
        collect_package "$dep" || return 1
      done <<< "$deps"
      ;;
    1) return 0 ;;
    *)
      echo "error: failed to read package composition for $pkg" >&2
      return 1
      ;;
  esac
}

copy_package() {
  local name="$1" role="$2" src dest
  src="$(package_root_for_name "$name")" || return 1
  dest="$work/packages/$name"
  mkdir -p "$dest"
  case "$role" in
    normal)
      (cd "$src" && LC_ALL=C tar --exclude './tests/run_graph*_test.lua' -cf - .) \
        | (cd "$dest" && LC_ALL=C tar xf -)
      ;;
    graph-target)
      (cd "$src" && LC_ALL=C tar --exclude './departments/test_*' --exclude './tests/*_test.lua' -cf - .) \
        | (cd "$dest" && LC_ALL=C tar xf -)
      if compgen -G "$src/tests/run_graph*_test.lua" >/dev/null; then
        mkdir -p "$dest/tests"
        cp "$src"/tests/run_graph*_test.lua "$dest/tests/"
      fi
      ;;
    graph-dep)
      (cd "$src" && LC_ALL=C tar --exclude './departments/test_*' --exclude './tests' -cf - .) \
        | (cd "$dest" && LC_ALL=C tar xf -)
      ;;
  esac
}

copy_libraries() {
  local lib dest
  mkdir -p "$work/libraries"
  for lib in "$ROOT/libraries"/*; do
    [ -d "$lib" ] || continue
    dest="$work/libraries/$(basename "$lib")"
    mkdir -p "$dest"
    (cd "$lib" && LC_ALL=C tar -cf - .) | (cd "$dest" && LC_ALL=C tar xf -)
  done
}

write_workspace() {
  cat > "$work/fkst.workspace.toml" <<'TOML'
[workspace]
units = ["packages/*", "libraries/*"]
packages = ["packages/*"]
libraries = ["libraries/*"]

[registries]
workspace = "workspace"
TOML
}

if [ "$#" -ne 2 ]; then
  usage
  exit 2
fi

mode="$1"
target="$2"
package_root_for_name "$target" >/dev/null || {
  echo "error: package not found: $target" >&2
  exit 1
}

work_parent="${FKST_RUNTIME_ROOT:-${TMPDIR:-/tmp}}"
work="$(mktemp -d "$work_parent/fkst-composed-test.XXXXXX")"
mkdir -p "$work/packages"
copy_libraries
write_workspace

case "$mode" in
  normal)
    copy_package "$target" normal
    write_roots "$work/packages/$target" "$work/packages/$target"
    ;;
  graph)
    seen=()
    collect_package "$target"
    roots=()
    for name in "${seen[@]}"; do
      if [ "$name" = "$target" ]; then
        copy_package "$name" graph-target
      else
        copy_package "$name" graph-dep
      fi
      roots+=("$work/packages/$name")
    done
    write_roots "$work" "${roots[@]}"
    ;;
  *)
    usage
    exit 2
    ;;
esac
