#!/usr/bin/env bash

set -euo pipefail

usage() {
  printf '%s\n' 'Usage: scripts/run.sh test | scripts/run.sh test <backend|frontend|local-qa-runtime|qa-contracts> | scripts/run.sh test-affected' >&2
}

script_dir=$(CDPATH= cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)
repository_root=$(CDPATH= cd -- "$script_dir/.." && pwd -P)

run_backend() {
  cd "$repository_root/backend"

  cargo fmt --all -- --check
  cargo clippy --workspace --all-targets -- -D warnings
  cargo build --workspace --locked
  cargo test --workspace --locked
}

run_frontend() {
  cd "$repository_root/frontend"

  npm ci
  npm run lint
  npm run typecheck
  npm run test
  npm run build
}

run_local_qa_rust() {
  cd "$repository_root/apps/local-qa-runtime"

  cargo fmt --all -- --check
  cargo clippy --workspace --all-targets --locked -- -D warnings
  cargo build --workspace --locked
  cargo test --workspace --locked
}

run_local_qa_workers() {
  cd "$repository_root/apps/local-qa-runtime/workers"

  npm ci --ignore-scripts
  npm run --ignore-scripts typecheck
  npm run --ignore-scripts build
  npm run --ignore-scripts test
}

run_local_qa_scaffold() {
  cd "$repository_root"
  bash apps/local-qa-runtime/tests/scaffold-structure.sh
}

run_qa_contracts_typescript() {
  cd "$repository_root/packages/qa-contracts"

  npm ci --ignore-scripts
  npm run --ignore-scripts typecheck
  npm run --ignore-scripts build
  npm run --ignore-scripts test
}

run_local_qa_runtime() {
  run_qa_contracts_typescript
  run_local_qa_rust
  run_local_qa_workers
  run_local_qa_scaffold
}

run_qa_contracts() {
  run_qa_contracts_typescript
  run_local_qa_rust
  run_local_qa_workers
}

run_full_suite() {
  run_backend
  run_frontend
  run_local_qa_runtime
}

resolve_ref_oid() {
  local ref=$1
  local local_only=$2
  local oid
  local status
  local fetch_ref

  # The selected ref is printed in the summary; control characters cannot be data there.
  if [[ "$ref" == -* || "$ref" == *[[:cntrl:]]* ]]; then
    return 1
  fi
  if oid=$(git -C "$repository_root" rev-parse --verify --end-of-options "$ref^{commit}" 2>/dev/null); then
    printf '%s\n' "$oid"
    return 0
  else
    status=$?
  fi
  [[ "$local_only" == true ]] && return "$status"

  case "$ref" in
    refs/heads/*) fetch_ref=${ref#refs/heads/} ;;
    refs/remotes/origin/*) fetch_ref=${ref#refs/remotes/origin/} ;;
    origin/*) fetch_ref=${ref#origin/} ;;
    refs/*) return "$status" ;;
    *) fetch_ref=$ref ;;
  esac
  # Local commit expressions may resolve above, but only branch names may be fetched.
  if [[ "$fetch_ref" == HEAD || "$fetch_ref" == -* ]] ||
    ! git -C "$repository_root" check-ref-format "refs/heads/$fetch_ref" >/dev/null 2>&1; then
    return "$status"
  fi

  git -C "$repository_root" fetch --no-tags --quiet origin \
    "refs/heads/$fetch_ref:refs/remotes/origin/$fetch_ref" >/dev/null 2>&1 || return $?
  oid=$(git -C "$repository_root" rev-parse --verify --end-of-options \
    "refs/remotes/origin/$fetch_ref^{commit}" 2>/dev/null) || return $?
  printf '%s\n' "$oid"
}

resolve_integration_base() {
  local candidate
  local local_only=false
  local status

  if [[ -n "${FKST_DEVLOOP_INTEGRATION_BRANCH:-}" ]]; then
    candidate=$FKST_DEVLOOP_INTEGRATION_BRANCH
  elif [[ -n "${GITHUB_BASE_REF:-}" ]]; then
    candidate=$GITHUB_BASE_REF
  else
    candidate=origin/HEAD
    local_only=true
  fi

  set +e
  integration_base_oid=$(resolve_ref_oid "$candidate" "$local_only")
  status=$?
  set -e
  if [[ "$status" -ne 0 ]]; then
    printf '%s\n' 'Unable to resolve integration base' >&2
    return "$status"
  fi

  resolved_integration_base=$candidate
}

select_affected_mode() {
  local changed_path
  local area=''
  local current_area
  local changed_paths_file=$1

  if [[ ! -s "$changed_paths_file" ]]; then
    affected_mode=full
    affected_rationale=root-or-ambiguous
    return 0
  fi

  while IFS= read -r -d '' changed_path; do
    case "$changed_path" in
      backend/*) current_area=backend ;;
      frontend/*) current_area=frontend ;;
      apps/local-qa-runtime/*) current_area=local-qa-runtime ;;
      packages/qa-contracts/*) current_area=qa-contracts ;;
      *)
        affected_mode=full
        affected_rationale=root-or-ambiguous
        return 0
        ;;
    esac

    if [[ -z "$area" ]]; then
      area=$current_area
    elif [[ "$area" != "$current_area" ]]; then
      affected_mode=full
      affected_rationale=multi-area
      return 0
    fi
  done <"$changed_paths_file"

  affected_mode=$area
  affected_rationale=single-area
}

run_affected_suite() {
  local changed_paths_file=$1
  local merge_base
  local status

  resolve_integration_base || return $?

  set +e
  merge_base=$(git -C "$repository_root" merge-base "$integration_base_oid" HEAD 2>/dev/null)
  status=$?
  set -e
  if [[ "$status" -ne 0 ]]; then
    return "$status"
  fi

  set +e
  git -C "$repository_root" diff --name-only -z --no-renames --diff-filter=ACDMRTUXB "$merge_base" HEAD -- >"$changed_paths_file" 2>/dev/null
  status=$?
  set -e
  if [[ "$status" -ne 0 ]]; then
    return "$status"
  fi

  select_affected_mode "$changed_paths_file"
  printf 'mode=%s base=%s rationale=%s\n' "$affected_mode" "$resolved_integration_base" "$affected_rationale"

  case "$affected_mode" in
    backend) run_backend ;;
    frontend) run_frontend ;;
    local-qa-runtime) run_local_qa_runtime ;;
    qa-contracts) run_qa_contracts ;;
    full) run_full_suite ;;
  esac
}

if [[ "$#" -eq 1 && "${1:-}" == 'test' ]]; then
  run_full_suite
elif [[ "$#" -eq 2 && "${1:-}" == 'test' ]]; then
  case "$2" in
    backend) run_backend ;;
    frontend) run_frontend ;;
    local-qa-runtime) run_local_qa_runtime ;;
    qa-contracts) run_qa_contracts ;;
    *) usage; exit 1 ;;
  esac
elif [[ "$#" -eq 1 && "${1:-}" == 'test-affected' ]]; then
  changed_paths_file=$(mktemp)
  cleanup() { rm -f -- "$changed_paths_file"; }
  trap cleanup EXIT
  run_affected_suite "$changed_paths_file"
else
  usage
  exit 1
fi
