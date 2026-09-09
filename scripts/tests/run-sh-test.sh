#!/usr/bin/env bash

set -euo pipefail

script_dir=$(CDPATH= cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)
repository_root=$(CDPATH= cd -- "$script_dir/../.." && pwd -P)
run_script="$repository_root/scripts/run.sh"
# Capture the running interpreter before installing the fake PATH wrappers.
export REAL_BASH="$BASH"
export REAL_GIT="$(command -v git)"

temporary_directory=$(mktemp -d)
temporary_directory=$(CDPATH= cd -- "$temporary_directory" && pwd -P)
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
mkdir -p -- "$fake_bin" "$temporary_directory/scratch"
export EXPECTED_GIT_ROOT="$repository_root"
export GIT_FETCH_MARKER="$temporary_directory/fetched"
export SECRET_SENTINEL='dispatcher-secret-sentinel'
unset FKST_DEVLOOP_INTEGRATION_BRANCH GITHUB_BASE_REF FAIL_COMMAND_AT FAIL_CLIPPY \
  FAIL_NPM_COMMAND FAIL_BASH_STATUS FAIL_GIT_STAGE REAL_GIT_FIXTURE

cat >"$fake_bin/cargo" <<'EOF_FAKE_CARGO'
#!/usr/bin/env bash

set -euo pipefail

printf '%s\t%s\n' "$PWD" "$*" >> "$CARGO_LOG"
if [[ -n "${COMMAND_LOG:-}" ]]; then
  printf '%s\t%s\n' "$PWD" "$*" >> "$COMMAND_LOG"
  if [[ -n "${FAIL_COMMAND_AT:-}" && $(wc -l < "$COMMAND_LOG") -eq "$FAIL_COMMAND_AT" ]]; then
    exit 77
  fi
fi
if [[ "${FAIL_CLIPPY:-}" == '1' && "$1" == 'clippy' ]]; then
  exit 37
fi
EOF_FAKE_CARGO
chmod +x "$fake_bin/cargo"

cat >"$fake_bin/npm" <<'EOF_FAKE_NPM'
#!/usr/bin/env bash

set -euo pipefail

printf '%s\t%s\n' "$PWD" "$*" >> "$NPM_LOG"
if [[ -n "${COMMAND_LOG:-}" ]]; then
  printf '%s\t%s\n' "$PWD" "$*" >> "$COMMAND_LOG"
  if [[ -n "${FAIL_COMMAND_AT:-}" && $(wc -l < "$COMMAND_LOG") -eq "$FAIL_COMMAND_AT" ]]; then
    exit 77
  fi
fi
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

  assert_equal 1 "$status" "$description usage status"
  assert_equal '' "$(cat "$temporary_directory/stdout")" "$description usage stdout"
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
#!/bin/sh
set -eu
if [ "${1:-}" = 'apps/local-qa-runtime/tests/scaffold-structure.sh' ]; then
  printf '%s\t%s\n' "$PWD" "$*" >> "$BASH_LOG"
  if [ -n "${COMMAND_LOG:-}" ]; then
    printf '%s\t%s\n' "$PWD" "$*" >> "$COMMAND_LOG"
    if [ -n "${FAIL_COMMAND_AT:-}" ] && [ "$(wc -l < "$COMMAND_LOG")" -eq "$FAIL_COMMAND_AT" ]; then
      exit 77
    fi
  fi
  if [ -n "${FAIL_BASH_STATUS:-}" ]; then
    exit "$FAIL_BASH_STATUS"
  fi
  exit 0
fi
exec "$REAL_BASH" "$@"
EOF_FAKE_BASH
chmod +x "$fake_bin/bash"

cat >"$fake_bin/git" <<'EOF_FAKE_GIT'
#!/usr/bin/env bash
set -euo pipefail
printf '%s\n' "$*" >> "$GIT_LOG"
if [[ "$1" != -C || "$2" != "$EXPECTED_GIT_ROOT" ]]; then
  printf '%s\n' 'Git must use the script repository root' >&2
  exit 45
fi
if [[ "${REAL_GIT_FIXTURE:-}" == 1 ]]; then exec "$REAL_GIT" "$@"; fi
shift 2
case "$1" in
  rev-parse)
    case "${FAIL_GIT_STAGE:-}" in
      ref|fetch) exit 41 ;;
      fetch-success|final-verify)
        if [[ ! -e "$GIT_FETCH_MARKER" ]]; then exit 41; fi
        if [[ "$FAIL_GIT_STAGE" == final-verify ]]; then exit 46; fi
        ;;
    esac
    printf '%s\n' fake-integration-oid
    ;;
  check-ref-format)
    exec "$REAL_GIT" "$@"
    ;;
  fetch)
    printf '%s\n' "$SECRET_SENTINEL"
    printf '%s\n' "$SECRET_SENTINEL" >&2
    if [[ "${FAIL_GIT_STAGE:-}" == 'fetch' ]]; then exit 42; fi
    : > "$GIT_FETCH_MARKER"
    ;;
  merge-base)
    if [[ "${FAIL_GIT_STAGE:-}" == 'merge-base' ]]; then exit 43; fi
    printf '%s\n' fake-merge-base
    ;;
  diff)
    [[ "$*" == 'diff --name-only -z --no-renames --diff-filter=ACDMRTUXB fake-merge-base HEAD --' ]] || exit 45
    printf '%b' "${GIT_CHANGED_PATHS:-}"
    if [[ "${FAIL_GIT_STAGE:-}" == 'diff' ]]; then
      printf '%s\n' "$SECRET_SENTINEL" >&2
      exit 44
    fi
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

expected_runtime_log=$(printf '%s\n%s\n%s\n%s' "$expected_qa_contracts_log" "$expected_local_qa_rust_log" "$expected_local_qa_workers_log" "$expected_scaffold_log")
expected_contracts_group_log=$(printf '%s\n%s\n%s' "$expected_qa_contracts_log" "$expected_local_qa_rust_log" "$expected_local_qa_workers_log")
expected_full_log=$(printf '%s\n%s\n%s' "$expected_log" "$expected_frontend_log" "$expected_runtime_log")
test_group_dispatch 'local-qa-runtime' "$expected_runtime_log"
test_group_dispatch 'qa-contracts' "$expected_contracts_group_log"
test_group_dispatch 'full' "$expected_full_log"

clear_logs() {
  : >"$log_file"
  : >"$frontend_log_file"
  : >"$bash_log_file"
  : >"$git_log_file"
  : >"$command_log_file"
  rm -f -- "$GIT_FETCH_MARKER"
}

invoke_mock() (
  cd "${CALLER_CWD:-$temporary_directory}"
  export PATH="$fake_bin:$PATH" CARGO_LOG="$log_file" NPM_LOG="$frontend_log_file" \
    BASH_LOG="$bash_log_file" COMMAND_LOG="$command_log_file" GIT_LOG="$git_log_file" \
    TMPDIR="$temporary_directory/scratch"
  "$run_script" "$@"
)

assert_cleanup_and_no_leak() {
  local leftovers
  leftovers=$(find "$temporary_directory/scratch" -type f -print)
  assert_equal '' "$leftovers" 'affected temporary file cleanup'
  if grep -Fq "$SECRET_SENTINEL" "$temporary_directory/affected-output" "$temporary_directory/affected-error"; then
    printf '%s\n' 'FAIL: sentinel leaked to stdout/stderr' >&2
    exit 1
  fi
}

run_affected_case() {
  local changed_paths=$1
  local expected_mode=$2
  local expected_rationale=$3
  local expected_dispatch=$4
  local expected_base=${5:-feature-branch}

  clear_logs
  GIT_CHANGED_PATHS="$changed_paths" FKST_DEVLOOP_INTEGRATION_BRANCH="$expected_base" \
    invoke_mock test-affected >"$temporary_directory/affected-output" 2>"$temporary_directory/affected-error"
  assert_equal "mode=$expected_mode base=$expected_base rationale=$expected_rationale" \
    "$(cat "$temporary_directory/affected-output")" "affected summary for $changed_paths"
  assert_equal "$expected_dispatch" "$(cat "$command_log_file")" "affected dispatch for $changed_paths"
  if grep -q 'checkout\|reset\|switch' "$git_log_file"; then
    printf 'FAIL: affected selection mutated checkout\n' >&2
    exit 1
  fi
  assert_cleanup_and_no_leak
}

run_affected_case 'backend/src/lib.rs\0' backend single-area "$expected_log"
run_affected_case 'frontend/src/app.ts\0' frontend single-area "$expected_frontend_log"
run_affected_case 'apps/local-qa-runtime/src/lib.rs\0' local-qa-runtime single-area "$expected_runtime_log"
run_affected_case 'packages/qa-contracts/src/index.ts\0' qa-contracts single-area "$expected_contracts_group_log"
run_affected_case 'backend/src/lib.rs\0frontend/src/app.ts\0' full multi-area "$expected_full_log"
run_affected_case 'README.md\0' full root-or-ambiguous "$expected_full_log"
run_affected_case '' full root-or-ambiguous "$expected_full_log"
run_affected_case 'backend/space name\0backend/tab\tname\0backend/new\nline\0backend/quote"\\name\0' \
  backend single-area "$expected_log"

# Every command is a possible first failure, including duplicate npm commands in
# different directories. The exact prefix proves failures cannot reach Workers.
test_failure_prefixes() {
  local mode=$1
  local commands=$2
  local affected_paths=$3
  local route command prefix='' index=0 status
  for route in direct affected; do
    prefix=''
    index=0
    while IFS= read -r command; do
      index=$((index + 1))
      prefix+="$command"$'\n'
      clear_logs
      set +e
      if [[ "$route" == affected ]]; then
        FAIL_COMMAND_AT="$index" GIT_CHANGED_PATHS="$affected_paths" FKST_DEVLOOP_INTEGRATION_BRANCH=feature-branch \
          invoke_mock test-affected >"$temporary_directory/affected-output" 2>"$temporary_directory/affected-error"
      elif [[ "$mode" == full ]]; then
        FAIL_COMMAND_AT="$index" invoke_mock test >"$temporary_directory/affected-output" 2>"$temporary_directory/affected-error"
      else
        FAIL_COMMAND_AT="$index" invoke_mock test "$mode" >"$temporary_directory/affected-output" 2>"$temporary_directory/affected-error"
      fi
      status=$?
      set -e
      assert_equal 77 "$status" "$route $mode failure $index status"
      assert_equal "${prefix%$'\n'}" "$(cat "$command_log_file")" "$route $mode failure $index prefix"
      if [[ "$route" == affected ]]; then
        local rationale=single-area
        [[ "$mode" == full ]] && rationale=root-or-ambiguous
        assert_equal "mode=$mode base=feature-branch rationale=$rationale" \
          "$(cat "$temporary_directory/affected-output")" "$mode dispatch failure keeps successful selection summary"
      else
        assert_equal '' "$(cat "$temporary_directory/affected-output")" 'direct failure stdout'
      fi
      assert_cleanup_and_no_leak
    done <<< "$commands"
  done
}
test_failure_prefixes backend "$expected_log" 'backend/a\0'
test_failure_prefixes frontend "$expected_frontend_log" 'frontend/a\0'
test_failure_prefixes local-qa-runtime "$expected_runtime_log" 'apps/local-qa-runtime/a\0'
test_failure_prefixes qa-contracts "$expected_contracts_group_log" 'packages/qa-contracts/a\0'
test_failure_prefixes full "$expected_full_log" 'scripts/run.sh\0'

# Nonempty precedence, empty values, and the completely unset default.
for selection in integration github empty unset; do
  clear_logs
  (
    export GIT_CHANGED_PATHS='backend/a\0'
    case "$selection" in
      integration) export FKST_DEVLOOP_INTEGRATION_BRANCH=feature-branch GITHUB_BASE_REF=base-branch ;;
      github) export FKST_DEVLOOP_INTEGRATION_BRANCH='' GITHUB_BASE_REF=base-branch ;;
      empty) export FKST_DEVLOOP_INTEGRATION_BRANCH='' GITHUB_BASE_REF='' ;;
      unset) unset FKST_DEVLOOP_INTEGRATION_BRANCH GITHUB_BASE_REF ;;
    esac
    invoke_mock test-affected
  ) >"$temporary_directory/affected-output" 2>"$temporary_directory/affected-error"
  selected=origin/HEAD
  [[ "$selection" == integration ]] && selected=feature-branch
  [[ "$selection" == github ]] && selected=base-branch
  assert_equal "mode=backend base=$selected rationale=single-area" "$(cat "$temporary_directory/affected-output")" "$selection precedence"
  assert_equal "$(printf '%s\n%s\n%s' \
    "-C $repository_root rev-parse --verify --end-of-options $selected^{commit}" \
    "-C $repository_root merge-base fake-integration-oid HEAD" \
    "-C $repository_root diff --name-only -z --no-renames --diff-filter=ACDMRTUXB fake-merge-base HEAD --")" \
    "$(cat "$git_log_file")" "$selection exact Git commands"
  assert_cleanup_and_no_leak
done

# Missing branches are normalized into one explicit branch-to-tracking refspec.
for selected in feature-branch origin/feature-branch refs/heads/feature-branch refs/remotes/origin/feature-branch; do
  FAIL_GIT_STAGE=fetch-success run_affected_case 'backend/a\0' backend single-area "$expected_log" "$selected"
  assert_equal "$(printf '%s\n%s\n%s\n%s\n%s\n%s' \
    "-C $repository_root rev-parse --verify --end-of-options $selected^{commit}" \
    "-C $repository_root check-ref-format refs/heads/feature-branch" \
    "-C $repository_root fetch --no-tags --quiet origin refs/heads/feature-branch:refs/remotes/origin/feature-branch" \
    "-C $repository_root rev-parse --verify --end-of-options refs/remotes/origin/feature-branch^{commit}" \
    "-C $repository_root merge-base fake-integration-oid HEAD" \
    "-C $repository_root diff --name-only -z --no-renames --diff-filter=ACDMRTUXB fake-merge-base HEAD --")" \
    "$(cat "$git_log_file")" "$selected normalized fetch"
done

for git_stage in ref fetch final-verify merge-base diff; do
  clear_logs
  set +e
  # Supplying partial diff bytes before failure must never produce a summary.
  FAIL_GIT_STAGE="$git_stage" FKST_DEVLOOP_INTEGRATION_BRANCH=feature-branch GITHUB_BASE_REF=unrelated-base \
    GIT_CHANGED_PATHS='backend/partial\0' invoke_mock test-affected \
    >"$temporary_directory/affected-output" 2>"$temporary_directory/affected-error"
  status=$?
  set -e
  case "$git_stage" in
    ref) expected_status=41 ;;
    fetch) expected_status=42 ;;
    final-verify) expected_status=46 ;;
    merge-base) expected_status=43 ;;
    diff) expected_status=44 ;;
  esac
  assert_equal "$expected_status" "$status" "Git $git_stage status"
  assert_equal '' "$(cat "$temporary_directory/affected-output")" "no summary after Git $git_stage failure"
  assert_equal '' "$(cat "$command_log_file")" "no dispatch after Git $git_stage failure"
  if grep -q 'unrelated-base\|origin/HEAD' "$git_log_file"; then
    printf '%s\n' 'FAIL: failed selected base fell back' >&2
    exit 1
  fi
  assert_cleanup_and_no_leak
done

for selection in empty unset; do
  clear_logs
  set +e
  (
    export FAIL_GIT_STAGE=ref
    if [[ "$selection" == empty ]]; then
      export FKST_DEVLOOP_INTEGRATION_BRANCH='' GITHUB_BASE_REF=''
    else
      unset FKST_DEVLOOP_INTEGRATION_BRANCH GITHUB_BASE_REF
    fi
    invoke_mock test-affected
  ) >"$temporary_directory/affected-output" 2>"$temporary_directory/affected-error"
  status=$?
  set -e
  assert_equal 41 "$status" "$selection missing origin/HEAD status"
  assert_equal "-C $repository_root rev-parse --verify --end-of-options origin/HEAD^{commit}" \
    "$(cat "$git_log_file")" 'default is local only'
  assert_equal '' "$(cat "$command_log_file" "$temporary_directory/affected-output")" 'missing default has no dispatch or summary'
  assert_cleanup_and_no_leak
done

# Real Git fixtures exercise quoting, renames, branch validation, and caller cwd.
# No checkout data or config is copied except the script under test.
(
  unset GIT_DIR GIT_WORK_TREE GIT_INDEX_FILE GIT_COMMON_DIR GIT_OBJECT_DIRECTORY GIT_ALTERNATE_OBJECT_DIRECTORIES GIT_CONFIG_COUNT GIT_CONFIG_PARAMETERS
  export GIT_CONFIG_NOSYSTEM=1 GIT_CONFIG_GLOBAL=/dev/null GIT_TERMINAL_PROMPT=0 GIT_ALLOW_PROTOCOL=file
  fixture="$temporary_directory/repository"
  unrelated="$temporary_directory/unrelated"
  mkdir -p "$fixture/scripts" "$fixture/backend" "$fixture/frontend" \
    "$fixture/apps/local-qa-runtime/workers" "$fixture/packages/qa-contracts" "$unrelated"
  cp "$run_script" "$fixture/scripts/run.sh"
  run_script="$fixture/scripts/run.sh"
  export EXPECTED_GIT_ROOT="$fixture" REAL_GIT_FIXTURE=1
  fixture_git() { "$REAL_GIT" -C "$fixture" "$@"; }
  fixture_git init -q
  fixture_git config user.name 'Dispatcher Test'
  fixture_git config user.email 'dispatcher@example.invalid'
  fixture_git config commit.gpgsign false
  fixture_git config core.hooksPath /dev/null
  printf '%s\n' baseline >"$fixture/backend/original"
  printf '%s\n' baseline >"$fixture/backend/deleted"
  fixture_git add -- backend scripts
  fixture_git commit -qm baseline
  fixture_git branch fixture-base
  base_oid=$(fixture_git rev-parse HEAD)
  "$REAL_GIT" -C "$unrelated" init -q
  "$REAL_GIT" -C "$unrelated" -c user.name='Dispatcher Test' -c user.email=dispatcher@example.invalid \
    -c commit.gpgsign=false -c core.hooksPath=/dev/null commit --allow-empty -qm unrelated
  fixture_backend_log=${expected_log//"$repository_root"/"$fixture"}
  fixture_full_log=${expected_full_log//"$repository_root"/"$fixture"}

  for name in 'space name' $'tab\tname' $'new\nline' 'quote"name' 'back\slash' 'unicode-é' '-leading'; do
    printf '%s\n' "$SECRET_SENTINEL" >"$fixture/backend/$name"
  done
  rm -- "$fixture/backend/deleted"
  fixture_git add -- backend
  fixture_git commit -qm unusual-paths-and-deletion
  CALLER_CWD="$unrelated" run_affected_case '' backend single-area "$fixture_backend_log" fixture-base
  CALLER_CWD="$temporary_directory" run_affected_case '' backend single-area "$fixture_backend_log" "$base_oid"
  run_affected_case '' backend single-area "$fixture_backend_log" 'HEAD~1'
  fixture_git tag fixture-tag "$base_oid"
  run_affected_case '' backend single-area "$fixture_backend_log" refs/tags/fixture-tag
  fixture_git update-ref refs/remotes/origin/default "$base_oid"
  fixture_git symbolic-ref refs/remotes/origin/HEAD refs/remotes/origin/default
  run_affected_case '' backend single-area "$fixture_backend_log" origin/HEAD
  run_affected_case '' backend single-area "$fixture_backend_log" refs/remotes/origin/HEAD
  run_affected_case '' full root-or-ambiguous "$fixture_full_log" HEAD

  # Uncommitted changes remain outside the established merge-base..HEAD scope.
  printf '%s\n' uncommitted >"$fixture/frontend/uncommitted"
  fixture_git add -- frontend/uncommitted
  run_affected_case '' backend single-area "$fixture_backend_log" fixture-base
  fixture_git reset -q -- frontend/uncommitted

  # An identical cross-area rename needs both endpoints, even with rename config on.
  fixture_git config diff.renames true
  fixture_git mv -- backend/original frontend/renamed
  fixture_git commit -qm cross-area-rename
  run_affected_case '' full multi-area "$fixture_full_log" 'HEAD~1'
  fixture_git mv -- frontend/renamed root-renamed
  fixture_git commit -qm root-rename
  run_affected_case '' full root-or-ambiguous "$fixture_full_log" 'HEAD~1'

  # A local origin makes a real missing-ref fetch possible without network access.
  remote="$temporary_directory/origin.git"
  "$REAL_GIT" init --bare -q "$remote"
  fixture_git remote add origin "$remote"
  "$REAL_GIT" -C "$remote" fetch -q "$fixture" "$base_oid:refs/heads/remote-only"
  run_affected_case '' full root-or-ambiguous "$fixture_full_log" refs/heads/remote-only
  assert_equal "$base_oid" "$(fixture_git rev-parse refs/remotes/origin/remote-only)" 'real fetched tracking ref'
  assert_equal 1 "$(grep -c ' fetch --no-tags --quiet origin refs/heads/remote-only:refs/remotes/origin/remote-only$' "$git_log_file")" \
    'one real missing-ref fetch'

  for selected in '--output=stolen' $'fixture-base\nmode=forged' 'bad:refs/heads/stolen' \
    'bad*' 'bad..name' 'origin/-bad' 'HEAD~999' 'refs/tags/missing' \
    '$(touch INJECTION_SENTINEL)' "refs/heads/\`touch INJECTION_SENTINEL\`"; do
    clear_logs
    # Preserve real rev-parse failure status for unresolved refs that cannot be fetched.
    set +e
    fixture_git rev-parse --verify --end-of-options "$selected^{commit}" >/dev/null 2>&1
    expected_status=$?
    set -e
    [[ "$selected" == -* || "$selected" == *[[:cntrl:]]* ]] && expected_status=1
    set +e
    FKST_DEVLOOP_INTEGRATION_BRANCH="$selected" GITHUB_BASE_REF=fixture-base \
      invoke_mock test-affected >"$temporary_directory/affected-output" 2>"$temporary_directory/affected-error"
    status=$?
    set -e
    assert_equal "$expected_status" "$status" "invalid ref status: $selected"
    assert_equal '' "$(cat "$command_log_file" "$temporary_directory/affected-output")" 'invalid ref has no dispatch or summary'
    if grep -q ' fetch ' "$git_log_file" || [[ -e "$temporary_directory/INJECTION_SENTINEL" ]]; then
      printf '%s\n' 'FAIL: unsafe ref fetched or executed' >&2
      exit 1
    fi
    assert_cleanup_and_no_leak
  done
  fixture_git symbolic-ref --delete refs/remotes/origin/HEAD
  for selected in origin/HEAD refs/remotes/origin/HEAD; do
    clear_logs
    set +e
    FKST_DEVLOOP_INTEGRATION_BRANCH="$selected" invoke_mock test-affected \
      >"$temporary_directory/affected-output" 2>"$temporary_directory/affected-error"
    status=$?
    set -e
    assert_equal 128 "$status" 'missing explicit remote HEAD status'
    if grep -q ' fetch ' "$git_log_file"; then
      printf '%s\n' 'FAIL: fetched a remote HEAD branch' >&2
      exit 1
    fi
    assert_equal '' "$(cat "$command_log_file" "$temporary_directory/affected-output")" 'missing explicit HEAD has no dispatch or summary'
    assert_cleanup_and_no_leak
  done
)

assert_nonzero_with_usage 'extra affected argument' test-affected backend
assert_nonzero_with_usage 'extra full argument' test frontend extra
printf '%s\n' 'run-sh-test: PASS'
