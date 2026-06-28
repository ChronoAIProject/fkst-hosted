#!/usr/bin/env bash
# Changed-path local verification for implementation/fix worktrees.
#
# Scope is derived from the worktree's OWN uncommitted edits: the implement/fix
# codex makes its changes and runs local verification BEFORE the change is
# committed, so `git diff HEAD` + untracked files are exactly the codex's
# changes. This needs no base branch and no env var, so it is robust across
# branch topologies and across spawned-codex environments that do not carry
# FKST_DEVLOOP_INTEGRATION_BRANCH. CI runs the full `scripts/run.sh test` (all
# packages + composed conformance) as the comprehensive gate; this is fast local
# feedback only. When nothing scoped is detected (no uncommitted package edits),
# it falls back to the full suite.

test_affected_changed_paths() {
  {
    git -C "$ROOT" diff --name-only HEAD
    git -C "$ROOT" ls-files --others --exclude-standard
  } | sed '/^$/d' | LC_ALL=C sort -u
}

test_affected_is_root_config() {
  local path="$1"
  case "$path" in
    */*) return 1 ;;
    Cargo.toml|Cargo.lock|fkst.workspace.toml|fkst.lock|package.json|package-lock.json|pnpm-lock.yaml|yarn.lock|pyproject.toml|poetry.lock|requirements.txt|codecov.yml)
      return 0
      ;;
    *.toml|*.yml|*.yaml|*.lock|*.config.js|*.config.ts|*.config.cjs|*.config.mjs)
      return 0
      ;;
    *) return 1 ;;
  esac
}

test_affected_is_broad_path() {
  local path="$1"
  case "$path" in
    libraries/*|scripts/*|.github/*) return 0 ;;
  esac
  test_affected_is_root_config "$path"
}

test_affected_run_test() {
  if [ -n "${FKST_TEST_AFFECTED_RUNNER:-}" ]; then
    "$FKST_TEST_AFFECTED_RUNNER" "$@"
    return $?
  fi
  "$ROOT/scripts/run.sh" "$@"
}

cmd_test_affected() {
  local changed_file full=0 packages="" path package status=0
  changed_file="$(mktemp "${TMPDIR:-/tmp}/fkst-test-affected.XXXXXX")"
  test_affected_changed_paths > "$changed_file"

  while IFS= read -r path || [ -n "$path" ]; do
    [ -n "$path" ] || continue
    if test_affected_is_broad_path "$path"; then
      full=1
    fi
    case "$path" in
      packages/*/*)
        package="${path#packages/}"
        package="${package%%/*}"
        case " $packages " in
          *" $package "*) ;;
          *) packages="$packages $package" ;;
        esac
        ;;
    esac
  done < "$changed_file"
  rm -f "$changed_file"

  if [ "$full" -eq 1 ] || [ -z "${packages# }" ]; then
    test_affected_run_test test
    return $?
  fi
  for package in $packages; do
    if ! test_affected_run_test test "$package"; then
      status=1
    fi
  done
  return "$status"
}
