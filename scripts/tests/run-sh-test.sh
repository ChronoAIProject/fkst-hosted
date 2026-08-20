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
mkdir -p -- "$fake_bin"

cat >"$fake_bin/cargo" <<'EOF_FAKE_CARGO'
#!/usr/bin/env bash

set -euo pipefail

printf '%s\t%s\n' "$PWD" "$*" >> "$CARGO_LOG"
if [[ "${FAIL_CLIPPY:-}" == '1' && "$1" == 'clippy' ]]; then
  exit 37
fi
EOF_FAKE_CARGO
chmod +x "$fake_bin/cargo"

cat >"$fake_bin/npm" <<'EOF_FAKE_NPM'
#!/usr/bin/env bash

set -euo pipefail

printf '%s\t%s\n' "$PWD" "$*" >> "$NPM_LOG"
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
assert_nonzero_with_usage 'one argument' test
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
