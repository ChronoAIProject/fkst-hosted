#!/usr/bin/env bash

set -euo pipefail

script_dir=$(CDPATH= cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)
repository_root=$(CDPATH= cd -- "$script_dir/../.." && pwd -P)
run_script="$repository_root/scripts/run.sh"

temporary_directory=$(mktemp -d)
cleanup() {
  rm -rf -- "$temporary_directory"
}
trap cleanup EXIT

fake_bin="$temporary_directory/bin"
log_file="$temporary_directory/cargo.log"
frontend_log_file="$temporary_directory/npm.log"
bash_log_file="$temporary_directory/bash.log"
git_log_file="$temporary_directory/git.log"
command_log_file="$temporary_directory/commands.log"
mkdir -p -- "$fake_bin"

cat >"$fake_bin/cargo" <<'EOF_FAKE_CARGO'
#!/usr/bin/env bash

set -euo pipefail

printf '%s\t%s\n' "$PWD" "$*" >> "$CARGO_LOG"
if [[ -n "${COMMAND_LOG:-}" ]]; then printf '%s\t%s\n' "$PWD" "$*" >> "$COMMAND_LOG"; fi
if [[ "${FAIL_CLIPPY:-}" == '1' && "$1" == 'clippy' ]]; then
  exit 37
fi
EOF_FAKE_CARGO
chmod +x "$fake_bin/cargo"

cat >"$fake_bin/npm" <<'EOF_FAKE_NPM'
#!/usr/bin/env bash

set -euo pipefail

printf '%s\t%s\n' "$PWD" "$*" >> "$NPM_LOG"
if [[ -n "${COMMAND_LOG:-}" ]]; then printf '%s\t%s\n' "$PWD" "$*" >> "$COMMAND_LOG"; fi
if [[ "${FAIL_NPM_COMMAND:-}" == "$*" ]]; then
  exit "${FAIL_NPM_STATUS:-1}"
fi
EOF_FAKE_NPM
chmod +x "$fake_bin/npm"

assert_equal() {
  local expected=$1
  local actual=$2
  local description=$3

  if [[ "$expected" != "$actual" ]]; then
    printf 'FAIL: %s\nexpected: %q\nactual: %q\n' "$description" "$expected" "$actual" >&2
    exit 1
  fi
}

assert_nonzero_with_usage() {
  local description=$1
  shift
  local stderr_file="$temporary_directory/stderr"
  local status

  set +e
  "$run_script" "$@" 2>"$stderr_file" >"$temporary_directory/stdout"
  status=$?
  set -e

  if [[ "$status" -eq 0 ]]; then
    printf 'FAIL: %s unexpectedly succeeded\n' "$description" >&2
    exit 1
  fi
  printf '%s\n' 'Usage: scripts/run.sh test | scripts/run.sh test <backend|frontend|local-qa-runtime|qa-contracts> | scripts/run.sh test-affected' >"$temporary_directory/expected-stderr"
  if ! cmp -s "$temporary_directory/expected-stderr" "$stderr_file"; then
    printf 'FAIL: %s usage\n' "$description" >&2
    exit 1
  fi
}

expected_working_directory="$repository_root/backend"
(
  cd "$temporary_directory"
  PATH="$fake_bin:$PATH" CARGO_LOG="$log_file" "$run_script" test backend
)

expected_log=$(printf '%s\t%s\n%s\t%s\n%s\t%s\n%s\t%s' \
  "$expected_working_directory" 'fmt --all -- --check' \
  "$expected_working_directory" 'clippy --workspace --all-targets -- -D warnings' \
  "$expected_working_directory" 'build --workspace --locked' \
  "$expected_working_directory" 'test --workspace --locked')
assert_equal "$expected_log" "$(cat "$log_file")" 'backend dispatch'

assert_nonzero_with_usage 'missing arguments'
assert_nonzero_with_usage 'wrong first argument' backend
assert_nonzero_with_usage 'wrong backend argument' test frontend-invalid
assert_nonzero_with_usage 'extra argument' test backend extra

: >"$log_file"
set +e
(
  cd "$temporary_directory"
  PATH="$fake_bin:$PATH" CARGO_LOG="$log_file" FAIL_CLIPPY=1 "$run_script" test backend
)
status=$?
set -e
assert_equal '37' "$status" 'clippy failure status'
expected_clippy_log=$(printf '%s\t%s\n%s\t%s' \
  "$expected_working_directory" 'fmt --all -- --check' \
  "$expected_working_directory" 'clippy --workspace --all-targets -- -D warnings')
assert_equal "$expected_clippy_log" "$(cat "$log_file")" 'stop after clippy failure'

expected_frontend_working_directory="$repository_root/frontend"
frontend_commands=(
  'ci'
  'run lint'
  'run typecheck'
  'run test'
  'run build'
)

: >"$frontend_log_file"
(
  cd "$temporary_directory"
  PATH="$fake_bin:$PATH" NPM_LOG="$frontend_log_file" "$run_script" test frontend
)

expected_frontend_log=$(printf '%s\t%s\n' \
  "$expected_frontend_working_directory" 'ci' \
  "$expected_frontend_working_directory" 'run lint' \
  "$expected_frontend_working_directory" 'run typecheck' \
  "$expected_frontend_working_directory" 'run test' \
  "$expected_frontend_working_directory" 'run build')
assert_equal "$expected_frontend_log" "$(cat "$frontend_log_file")" 'frontend dispatch'

for frontend_command in "${frontend_commands[@]}"; do
  : >"$frontend_log_file"
  set +e
  (
    cd "$temporary_directory"
    PATH="$fake_bin:$PATH" \
      NPM_LOG="$frontend_log_file" \
      FAIL_NPM_COMMAND="$frontend_command" \
      FAIL_NPM_STATUS=73 \
      "$run_script" test frontend
  )
  status=$?
  set -e

  assert_equal '73' "$status" "frontend failure status for $frontend_command"

  expected_frontend_failure_log=''
  for expected_command in "${frontend_commands[@]}"; do
    expected_frontend_failure_log+="${expected_frontend_working_directory}"$'\t'"${expected_command}"$'\n'
    [[ "$expected_command" == "$frontend_command" ]] && break
  done
  expected_frontend_failure_log=${expected_frontend_failure_log%$'\n'}
  assert_equal "$expected_frontend_failure_log" "$(cat "$frontend_log_file")" \
    "stop after frontend failure for $frontend_command"
done

cat >"$fake_bin/bash" <<'EOF_FAKE_BASH'
#!/usr/bin/bash
set -euo pipefail
if [[ "$1" == 'apps/local-qa-runtime/tests/scaffold-structure.sh' ]]; then
  printf '%s\t%s\n' "$PWD" "$*" >> "$BASH_LOG"
  if [[ -n "${COMMAND_LOG:-}" ]]; then printf '%s\t%s\n' "$PWD" "$*" >> "$COMMAND_LOG"; fi
  if [[ -n "${FAIL_BASH_STATUS:-}" ]]; then
    exit "$FAIL_BASH_STATUS"
  fi
  exit 0
fi
exec /usr/bin/bash "$@"
EOF_FAKE_BASH
chmod +x "$fake_bin/bash"

cat >"$fake_bin/git" <<'EOF_FAKE_GIT'
#!/usr/bin/env bash
set -euo pipefail
printf '%s\n' "$*" >> "$GIT_LOG"
case "$1" in
  rev-parse)
    if [[ "${FAIL_GIT_STAGE:-}" == 'ref' ]]; then exit 41; fi
    if [[ "${FAIL_GIT_STAGE:-}" == 'fetch' ]]; then exit 41; fi
    printf '%s\n' fake-integration-oid
    ;;
  fetch)
    if [[ "${FAIL_GIT_STAGE:-}" == 'fetch' ]]; then exit 42; fi
    ;;
  merge-base)
    if [[ "${FAIL_GIT_STAGE:-}" == 'merge-base' ]]; then exit 43; fi
    printf '%s\n' fake-merge-base
    ;;
  diff)
    if [[ "${FAIL_GIT_STAGE:-}" == 'diff' ]]; then exit 44; fi
    printf '%b' "${GIT_CHANGED_PATHS:-}"
    ;;
  *)
    printf 'unexpected fake git command: %s\n' "$*" >&2
    exit 45
    ;;
esac
EOF_FAKE_GIT
chmod +x "$fake_bin/git"

test_group_dispatch() {
  local mode=$1
  local expected_log=$2
  local actual_log

  : >"$log_file"
  : >"$frontend_log_file"
  : >"$bash_log_file"
  : >"$command_log_file"
  (
    cd "$temporary_directory"
    export PATH="$fake_bin:$PATH" CARGO_LOG="$log_file" NPM_LOG="$frontend_log_file" \
      BASH_LOG="$bash_log_file" COMMAND_LOG="$command_log_file"
    if [[ "$mode" == full ]]; then
      "$run_script" test
    else
      "$run_script" test "$mode"
    fi
  )
  actual_log=$(cat "$command_log_file")
  assert_equal "$expected_log" "$actual_log" "$mode dispatch"
}

local_qa_rust_directory="$repository_root/apps/local-qa-runtime"
local_qa_workers_directory="$repository_root/apps/local-qa-runtime/workers"
qa_contracts_directory="$repository_root/packages/qa-contracts"
expected_local_qa_rust_log=$(printf '%s\t%s\n%s\t%s\n%s\t%s\n%s\t%s' \
  "$local_qa_rust_directory" 'fmt --all -- --check' \
  "$local_qa_rust_directory" 'clippy --workspace --all-targets --locked -- -D warnings' \
  "$local_qa_rust_directory" 'build --workspace --locked' \
  "$local_qa_rust_directory" 'test --workspace --locked')
expected_local_qa_workers_log=$(printf '%s\t%s\n%s\t%s\n%s\t%s\n%s\t%s' \
  "$local_qa_workers_directory" 'ci --ignore-scripts' \
  "$local_qa_workers_directory" 'run --ignore-scripts typecheck' \
  "$local_qa_workers_directory" 'run --ignore-scripts build' \
  "$local_qa_workers_directory" 'run --ignore-scripts test')
expected_scaffold_log=$(printf '%s\tapps/local-qa-runtime/tests/scaffold-structure.sh' "$repository_root")
expected_qa_contracts_log=$(printf '%s\t%s\n%s\t%s\n%s\t%s\n%s\t%s' \
  "$qa_contracts_directory" 'ci --ignore-scripts' \
  "$qa_contracts_directory" 'run --ignore-scripts typecheck' \
  "$qa_contracts_directory" 'run --ignore-scripts build' \
  "$qa_contracts_directory" 'run --ignore-scripts test')

test_group_dispatch 'local-qa-runtime' \
  "$(printf '%s\n%s\n%s' "$expected_local_qa_rust_log" "$expected_local_qa_workers_log" "$expected_scaffold_log")"
test_group_dispatch 'qa-contracts' \
  "$(printf '%s\n%s\n%s' "$expected_qa_contracts_log" "$expected_local_qa_rust_log" "$expected_local_qa_workers_log")"
test_group_dispatch 'full' \
  "$(printf '%s\n%s\n%s\n%s\n%s\n%s' "$expected_log" "$expected_frontend_log" "$expected_local_qa_rust_log" "$expected_local_qa_workers_log" "$expected_scaffold_log" "$expected_qa_contracts_log")"

run_affected_case() {
  local changed_paths=$1
  local expected_mode=$2
  local expected_rationale=$3
  local expected_dispatch=$4
  local expected_base=${5:-feature-branch}
  local output

  : >"$log_file"
  : >"$frontend_log_file"
  : >"$bash_log_file"
  : >"$git_log_file"
  : >"$command_log_file"
  output=$(
    cd "$temporary_directory"
    PATH="$fake_bin:$PATH" \
      CARGO_LOG="$log_file" \
      NPM_LOG="$frontend_log_file" \
      BASH_LOG="$bash_log_file" \
      COMMAND_LOG="$command_log_file" \
      GIT_LOG="$git_log_file" \
      GIT_CHANGED_PATHS="$changed_paths" \
      FKST_DEVLOOP_INTEGRATION_BRANCH="$expected_base" \
      "$run_script" test-affected
  )
  assert_equal "mode=$expected_mode base=$expected_base rationale=$expected_rationale" "$output" \
    "affected summary for $changed_paths"
  assert_equal "$expected_dispatch" "$(cat "$command_log_file")" \
    "affected dispatch for $changed_paths"
  if grep -q 'checkout\|reset\|switch' "$git_log_file"; then
    printf 'FAIL: affected selection mutated checkout\n' >&2
    exit 1
  fi
}

run_affected_case $'backend/src/lib.rs\n' backend single-area "$expected_log"
run_affected_case $'frontend/src/app.ts\n' frontend single-area "$expected_frontend_log"
run_affected_case $'apps/local-qa-runtime/src/lib.rs\n' local-qa-runtime single-area \
  "$(printf '%s\n%s\n%s' "$expected_local_qa_rust_log" "$expected_local_qa_workers_log" "$expected_scaffold_log")"
run_affected_case $'packages/qa-contracts/src/index.ts\n' qa-contracts single-area \
  "$(printf '%s\n%s\n%s' "$expected_qa_contracts_log" "$expected_local_qa_rust_log" "$expected_local_qa_workers_log")"
run_affected_case $'backend/src/lib.rs\nfrontend/src/app.ts\n' full multi-area \
  "$(printf '%s\n%s\n%s\n%s\n%s\n%s' "$expected_log" "$expected_frontend_log" "$expected_local_qa_rust_log" "$expected_local_qa_workers_log" "$expected_scaffold_log" "$expected_qa_contracts_log")"
run_affected_case $'README.md\n' full root-or-ambiguous \
  "$(printf '%s\n%s\n%s\n%s\n%s\n%s' "$expected_log" "$expected_frontend_log" "$expected_local_qa_rust_log" "$expected_local_qa_workers_log" "$expected_scaffold_log" "$expected_qa_contracts_log")"
run_affected_case '' full root-or-ambiguous \
  "$(printf '%s\n%s\n%s\n%s\n%s\n%s' "$expected_log" "$expected_frontend_log" "$expected_local_qa_rust_log" "$expected_local_qa_workers_log" "$expected_scaffold_log" "$expected_qa_contracts_log")"

assert_equal 'feature-branch' "$(
  : >"$git_log_file"
  PATH="$fake_bin:$PATH" GIT_LOG="$git_log_file" GIT_CHANGED_PATHS=$'backend/src/lib.rs\n' \
    FKST_DEVLOOP_INTEGRATION_BRANCH=feature-branch GITHUB_BASE_REF=base-branch \
    CARGO_LOG="$log_file" NPM_LOG="$frontend_log_file" BASH_LOG="$bash_log_file" \
    "$run_script" test-affected | sed 's/^.*base=//; s/ rationale=.*$//'
)" 'integration branch precedence'

: >"$log_file"
: >"$frontend_log_file"
: >"$bash_log_file"
for git_stage in ref fetch merge-base diff; do
  : >"$git_log_file"
  set +e
  (
    cd "$temporary_directory"
    PATH="$fake_bin:$PATH" \
      CARGO_LOG="$log_file" NPM_LOG="$frontend_log_file" BASH_LOG="$bash_log_file" GIT_LOG="$git_log_file" \
      FAIL_GIT_STAGE="$git_stage" FKST_DEVLOOP_INTEGRATION_BRANCH=feature-branch \
      "$run_script" test-affected >"$temporary_directory/affected-output" 2>"$temporary_directory/affected-error"
  )
  status=$?
  set -e
  expected_status=41
  [[ "$git_stage" == fetch ]] && expected_status=42
  [[ "$git_stage" == merge-base ]] && expected_status=43
  [[ "$git_stage" == diff ]] && expected_status=44
  assert_equal "$expected_status" "$status" "Git $git_stage status"
  assert_equal '' "$(cat "$temporary_directory/affected-output")" "no summary after Git $git_stage failure"
  assert_equal '' "$(cat "$log_file" "$frontend_log_file" "$bash_log_file")" "no dispatch after Git $git_stage failure"
done

printf '%s\n' 'run-sh-test: PASS'
