#!/bin/sh
set -eu

script_dir=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
source_dir=$(CDPATH='' cd -- "$script_dir/.." && pwd)
test_root=$(mktemp -d "${TMPDIR:-/tmp}/fkst-drill-test.XXXXXX")
trap 'rm -rf -- "$test_root"' EXIT HUP INT TERM

fixture="$test_root/fixture"
fake_bin="$test_root/bin"
log="$test_root/commands.log"
state="$test_root/state"
output="$test_root/output.log"
evidence="$test_root/evidence"
mkdir -p "$fixture/overlays/local" "$fake_bin"
cp "$source_dir/run-disaster-drill.sh" "$fixture/run-disaster-drill.sh"
cp "$source_dir/render-recovery-evidence.rb" "$fixture/render-recovery-evidence.rb"
touch "$fixture/overlays/local/kustomization.yaml"
chmod +x "$fixture/run-disaster-drill.sh"

cat >"$fixture/restore-namespace.sh" <<'RESTORE'
#!/bin/sh
set -eu
printf 'RESTORE %s\n' "$*" >>"$FAKE_KUBECTL_LOG"
case " $* " in
  *" --preflight-only "*) exit 0 ;;
esac
: >"$FAKE_KUBECTL_STATE.restored"
RESTORE
chmod +x "$fixture/restore-namespace.sh"

cat >"$fake_bin/kubectl" <<'KUBECTL'
#!/bin/sh
set -eu
printf 'KUBECTL %s\n' "$*" >>"$FAKE_KUBECTL_LOG"
[ "$#" -ge 3 ] && [ "$1" = "--context" ] || {
  echo "fake kubectl rejected an invocation without explicit context" >&2
  exit 91
}
[ "$2" = "$FAKE_EXPECTED_CONTEXT" ] || {
  echo "fake kubectl rejected the wrong context" >&2
  exit 92
}
shift 2
command_line=" $* "
case "$command_line" in
  *" config view "*) exit 0 ;;
  *" get namespace chronoai-fkst "*) printf '%s' "${FAKE_DISPOSABLE:-true}"; exit 0 ;;
  *" get namespace fkst-recovery-source "*) printf '%s' "${FAKE_DURABLE:-external}"; exit 0 ;;
  *" get pods "*)
    case "$command_line" in
      *"fkst-managed=true,fkst-owner=chronoai-shining,fkst-repo=chronoai-fkst-test"*) ;;
      *) echo "runtime query was not repository-scoped" >&2; exit 93 ;;
    esac
    if [ -e "$FAKE_KUBECTL_STATE.restored" ]; then
      printf '%b' "${FAKE_POST_SESSIONS:-session-b\tRunning\tactive\ttrue,true,\nsession-a\tRunning\tactive\ttrue,true,\n}"
    elif [ "${FAKE_LIVE:-true}" = "true" ]; then
      printf '%b' "${FAKE_PRE_SESSIONS:-session-a\tRunning\tactive\ttrue,true,\nsession-b\tRunning\tactive\ttrue,true,\n}"
    fi
    exit 0
    ;;
  *" get secret fkst-env-250120269-drill-sentinel "*)
    case "$command_line" in
      *"content-hash"*) printf '%s' "${FAKE_CONTENT_HASH:-0000000000000000000000000000000000000000000000000000000000000000}" ;;
      *"env-secret-keys"*) printf '["TOKEN","ZETA"]' ;;
      *"range"*) printf 'ciphertext\nnonce\n' ;;
      *) exit 94 ;;
    esac
    exit 0
    ;;
  *" get lease.coordination.k8s.io fkst-control-plane-reconciler "*)
    if [ -e "$FAKE_KUBECTL_STATE.restored" ]; then printf '1'; else printf '7'; fi
    exit 0
    ;;
  *" delete namespace chronoai-fkst "*)
    : >"$FAKE_KUBECTL_STATE.deleted"
    exit 0
    ;;
esac
echo "unhandled fake kubectl invocation: $*" >&2
exit 95
KUBECTL
chmod +x "$fake_bin/kubectl"

export PATH="$fake_bin:$PATH"
export FAKE_KUBECTL_LOG="$log"
export FAKE_KUBECTL_STATE="$state"
export FAKE_EXPECTED_CONTEXT="kind-opensandbox-local"
runner="$fixture/run-disaster-drill.sh"

reset_case() {
  : >"$log"
  rm -f -- "$state.deleted" "$state.restored"
  rm -rf -- "$evidence"
  FAKE_DISPOSABLE=true
  FAKE_DURABLE=external
  FAKE_LIVE=true
  FAKE_CONTENT_HASH=0000000000000000000000000000000000000000000000000000000000000000
  FAKE_PRE_SESSIONS='session-a\tRunning\tactive\ttrue,true,\nsession-b\tRunning\tactive\ttrue,true,\n'
  FAKE_POST_SESSIONS='session-b\tRunning\tactive\ttrue,true,\nsession-a\tRunning\tactive\ttrue,true,\n'
  export FAKE_DISPOSABLE FAKE_DURABLE FAKE_LIVE FAKE_CONTENT_HASH
  export FAKE_PRE_SESSIONS FAKE_POST_SESSIONS
}

invoke_drill() {
  set --
  if [ "$TEST_INCLUDE_CONTEXT" = "true" ]; then
    set -- "$@" --context "$TEST_CONTEXT"
  fi
  set -- "$@" \
    --target-namespace "$TEST_TARGET_NAMESPACE" \
    --confirm-delete "$TEST_CONFIRMATION" \
    --durable-namespace "$TEST_DURABLE_NAMESPACE" \
    --repository "$TEST_REPOSITORY" \
    --sentinel-user-id 250120269 \
    --sentinel-name drill-sentinel \
    --evidence-dir "$evidence" \
    --timeout-seconds 120
  "$runner" "$@"
}

reset_invocation() {
  TEST_INCLUDE_CONTEXT=true
  TEST_CONTEXT=kind-opensandbox-local
  TEST_TARGET_NAMESPACE=chronoai-fkst
  TEST_CONFIRMATION=chronoai-fkst
  TEST_DURABLE_NAMESPACE=fkst-recovery-source
  TEST_REPOSITORY=chronoai-shining/chronoai-fkst-test
}

expect_failure() {
  name=$1
  reset_case
  if invoke_drill >"$output" 2>&1; then
    echo "expected failure: $name" >&2
    exit 1
  fi
  if grep -Fq 'delete namespace' "$log"; then
    echo "safety gate deleted a namespace: $name" >&2
    exit 1
  fi
}

assert_no_delete() {
  if grep -Fq 'delete namespace' "$log"; then
    echo "a rejected drill reached namespace deletion" >&2
    exit 1
  fi
}

# Argument and context gates execute before any cluster mutation.
reset_invocation
TEST_INCLUDE_CONTEXT=false
expect_failure "missing context"
reset_invocation
TEST_CONTEXT=development
expect_failure "non-kind context"
reset_invocation
TEST_CONTEXT=kind-production
expect_failure "production-like context"
reset_invocation
TEST_CONFIRMATION=another-namespace
expect_failure "wrong confirmation"
reset_invocation
TEST_TARGET_NAMESPACE=another-namespace
TEST_CONFIRMATION=another-namespace
expect_failure "wrong target namespace"
reset_invocation
TEST_DURABLE_NAMESPACE=chronoai-fkst
expect_failure "same durability boundary"
reset_invocation
TEST_REPOSITORY=""
expect_failure "missing repository"
reset_invocation
TEST_REPOSITORY=chronoai-shining/extra/path
expect_failure "non-exact repository"

reset_invocation
reset_case
FAKE_DISPOSABLE=false
export FAKE_DISPOSABLE
if invoke_drill >"$output" 2>&1; then
  echo "expected disposable-label failure" >&2
  exit 1
fi
assert_no_delete

reset_invocation
reset_case
FAKE_CONTENT_HASH=not-a-content-hash
export FAKE_CONTENT_HASH
if invoke_drill >"$output" 2>&1; then
  echo "expected environment-sentinel failure" >&2
  exit 1
fi
assert_no_delete

reset_invocation
reset_case
FAKE_PRE_SESSIONS='session-a\tRunning\tactive\ttrue,true,\nsession-a\tRunning\tactive\ttrue,true,\n'
export FAKE_PRE_SESSIONS
if invoke_drill >"$output" 2>&1; then
  echo "expected duplicate-session failure" >&2
  exit 1
fi
assert_no_delete

reset_invocation
reset_case
FAKE_DURABLE=internal
export FAKE_DURABLE
if invoke_drill >"$output" 2>&1; then
  echo "expected durability-boundary failure" >&2
  exit 1
fi
assert_no_delete

reset_invocation
reset_case
FAKE_LIVE=false
export FAKE_LIVE
if invoke_drill >"$output" 2>&1; then
  echo "expected prepared-runtime failure" >&2
  exit 1
fi
assert_no_delete

# The successful fake-cluster run proves the one allowed delete, canonical
# restore invocation, deterministic ordering, and bounded evidence projection.
reset_invocation
reset_case
invoke_drill >"$output" 2>&1
[ -e "$state.deleted" ] && [ -e "$state.restored" ]
[ "$(grep -Fc 'delete namespace chronoai-fkst --wait=true --timeout=120s' "$log")" -eq 1 ]
[ "$(grep -Fc 'RESTORE --context kind-opensandbox-local' "$log")" -eq 2 ]
grep -Fq -- '--preflight-only' "$log"
grep -Fq -- '--sentinel-content-hash' "$log"

ruby -rjson -e '
  evidence = JSON.parse(File.read(ARGV.fetch(0)))
  expected = %w[
    evidence_version result failure_phase started_at completed_at rto_seconds
    context_class target_namespace repository_sha256 pre_session_count
    post_session_count pre_session_set_sha256 post_session_set_sha256
    environment_content_hash environment_secret_key_count
    environment_secret_keys_sha256 lease_transitions_before
    lease_transitions_after github_mutations
  ]
  abort "evidence schema drifted" unless evidence.keys == expected
  abort "drill did not pass" unless evidence["result"] == "passed"
  abort "session count drifted" unless evidence.values_at("pre_session_count", "post_session_count") == [2, 2]
  abort "session set drifted" unless evidence["pre_session_set_sha256"] == evidence["post_session_set_sha256"]
  abort "environment inventory drifted" unless evidence["environment_secret_key_count"] == 2
  abort "GitHub mutation count drifted" unless evidence["github_mutations"] == 0
  raw = File.read(ARGV.fetch(0))
  %w[chronoai-shining/chronoai-fkst-test session-a session-b TOKEN ZETA].each do |forbidden|
    abort "raw identity leaked into evidence" if raw.include?(forbidden)
  end
' "$evidence/recovery-evidence.json"

for artifact in "$evidence/recovery-evidence.json" "$evidence/recovery-evidence.md" "$output"; do
  if grep -Fq 'chronoai-shining/chronoai-fkst-test' "$artifact"; then
    echo "repository identity leaked into $artifact" >&2
    exit 1
  fi
  if grep -Eq 'session-[ab]' "$artifact"; then
    echo "session identity leaked into $artifact" >&2
    exit 1
  fi
done

if ruby "$source_dir/render-recovery-evidence.rb" --token forbidden >"$output" 2>&1; then
  echo "evidence renderer accepted an unbounded field" >&2
  exit 1
fi
if grep -Eq '(^|[[:space:]])gh[[:space:]]+(issue|api|pr)' "$source_dir/run-disaster-drill.sh"; then
  echo "drill runner contains a GitHub mutation path" >&2
  exit 1
fi

echo "disaster drill safety and redacted evidence tests passed"
