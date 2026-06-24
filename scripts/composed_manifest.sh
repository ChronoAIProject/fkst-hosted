#!/usr/bin/env bash
# Helpers for reading package composition from the engine manifest CLI.
#
# Contract:
#   is_composed: 0=composed, 1=flat, 2=error
#   composition_siblings_of: prints sibling package names and returns
#     0=composed, 1=flat, 2=error

composed_manifest_path() {
  local pkg="$1"
  printf '%s/fkst.toml\n' "$pkg"
}

composition_manifest_subcommand() {
  printf '%s%s%s\n' composed - deps
}

composition_siblings_of() {
  local pkg="$1" manifest output rc
  manifest="$(composed_manifest_path "$pkg")"
  if [ -z "${BIN:-}" ]; then
    echo "error: BIN is required for manifest composition query" >&2
    return 2
  fi
  set +e
  output="$("$BIN" manifest "$(composition_manifest_subcommand)" --manifest "$manifest")"
  rc=$?
  set -e
  case "$rc" in
    0)
      printf '%s\n' "$output"
      return 0
      ;;
    10)
      return 1
      ;;
    *)
      echo "error: manifest composition query failed for $manifest" >&2
      [ -n "$output" ] && printf '%s\n' "$output" >&2
      return 2
      ;;
  esac
}

is_composed() {
  local pkg="$1" output rc
  if [ -z "${BIN:-}" ]; then
    echo "error: BIN is required for manifest composition query" >&2
    return 2
  fi
  set +e
  output="$("$BIN" manifest "$(composition_manifest_subcommand)" --manifest "$(composed_manifest_path "$pkg")")"
  rc=$?
  set -e
  case "$rc" in
    0) return 0 ;;
    10) return 1 ;;
    *)
      echo "error: manifest composition query failed for $(composed_manifest_path "$pkg")" >&2
      [ -n "$output" ] && printf '%s\n' "$output" >&2
      return 2
      ;;
  esac
}
