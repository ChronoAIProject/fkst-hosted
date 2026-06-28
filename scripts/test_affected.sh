#!/usr/bin/env bash
# Changed-path local verification for implementation/fix worktrees.

test_affected_ref_exists() {
  git -C "$ROOT" rev-parse --verify --quiet "$1^{commit}" >/dev/null
}

test_affected_base_ref() {
  local branch="${FKST_DEVLOOP_INTEGRATION_BRANCH:-}"
  if [ -z "$branch" ]; then
    echo "error: FKST_DEVLOOP_INTEGRATION_BRANCH is required for test-affected" >&2
    return 1
  fi
  if ! git -C "$ROOT" check-ref-format --branch "$branch" >/dev/null 2>&1; then
    echo "error: invalid FKST_DEVLOOP_INTEGRATION_BRANCH: $branch" >&2
    return 1
  fi
  if test_affected_ref_exists "refs/remotes/origin/$branch"; then
    printf '%s\n' "refs/remotes/origin/$branch"
    return 0
  fi
  if test_affected_ref_exists "$branch"; then
    printf '%s\n' "$branch"
    return 0
  fi
  echo "error: integration branch ref not found: $branch" >&2
  return 1
}

test_affected_base_commit() {
  local base_ref="$1" fork_point
  if fork_point="$(git -C "$ROOT" merge-base --fork-point "$base_ref" HEAD 2>/dev/null)"; then
    printf '%s\n' "$fork_point"
    return 0
  fi
  git -C "$ROOT" merge-base "$base_ref" HEAD
}

test_affected_changed_paths() {
  local base_commit="$1"
  {
    git -C "$ROOT" diff --name-only "$base_commit"
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
  local base_ref base_commit changed_file full=0 packages="" path package status=0
  base_ref="$(test_affected_base_ref)" || return 1
  base_commit="$(test_affected_base_commit "$base_ref")" || return 1
  changed_file="$(mktemp "${TMPDIR:-/tmp}/fkst-test-affected.XXXXXX")"
  test_affected_changed_paths "$base_commit" > "$changed_file"

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
