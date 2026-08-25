#!/usr/bin/env bash
set -euo pipefail

root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)
cd "$root"

testing_packages_pin=apps/local-qa-runtime/.fkst/conformance/fkst-packages-testing.pin
expected_testing_packages_pin=$(mktemp)
trap 'rm -f "$expected_testing_packages_pin"' EXIT
printf 'ac953ff0bb3f1c909728e66c3968cbb3ed5e3cf1\n' > "$expected_testing_packages_pin"
[[ -f "$testing_packages_pin" ]] && cmp -s "$expected_testing_packages_pin" "$testing_packages_pin" || {
  echo 'Local QA Testing Packages pin must contain the exact immutable source revision' >&2
  exit 1
}

local_qa_readme=apps/local-qa-runtime/README.md
for required_reference in \
  'ChronoAIProject/fkst-packages-testing@ac953ff0bb3f1c909728e66c3968cbb3ed5e3cf1:packages/local-qa-host-adapter' \
  'ChronoAIProject/fkst-packages@d4146d7bbdbde9d6fbbee404d5a2e3e4da0fa08c' \
  'ChronoAIProject/fkst-substrate@e3355b42709f4138613b8238cba34a5ab1161053' \
  'testing-observation.v1' \
  'testing-assertion-result.v1' \
  'testing-case-result.v2' \
  'testing-case-result-set.v2' \
  'pinned but not activated'; do
  grep -Fq "$required_reference" "$local_qa_readme" || {
    echo "Local QA README is missing immutable Testing Packages reference: $required_reference" >&2
    exit 1
  }
done

expected=$(printf '%s\n' \
  apps/local-qa-runtime/browser-adapter/src/lib.rs \
  apps/local-qa-runtime/evidence-stager/src/lib.rs \
  apps/local-qa-runtime/evidence-stager/tests/browser_screenshot.rs \
  apps/local-qa-runtime/evidence-stager/tests/runner_log.rs \
  apps/local-qa-runtime/guest-agent/src/main.rs \
  apps/local-qa-runtime/host/src/coordinator.rs \
  apps/local-qa-runtime/host/src/executor.rs \
  apps/local-qa-runtime/host/src/journal.rs \
  apps/local-qa-runtime/host/src/lib.rs \
  apps/local-qa-runtime/host/src/main.rs \
  apps/local-qa-runtime/host/src/ownership.rs \
  apps/local-qa-runtime/host/tests/environment_ownership.rs \
  apps/local-qa-runtime/host/tests/fail_closed.rs \
  apps/local-qa-runtime/host/tests/loopback_sqlite.rs \
  apps/local-qa-runtime/launcher/src/main.rs \
  apps/local-qa-runtime/secret-broker/src/main.rs \
  apps/local-qa-runtime/supervisor/src/main.rs \
  apps/local-qa-runtime/workers/src/index.ts \
  apps/local-qa-runtime/workers/src/json.ts \
  apps/local-qa-runtime/workers/src/policy.ts \
  apps/local-qa-runtime/workers/src/protocol-worker.ts \
  apps/local-qa-runtime/workers/src/worker-error.ts \
  apps/local-qa-runtime/workers/src/worker-main.ts \
  apps/local-qa-runtime/workers/test/browser-smoke.test.mjs \
  apps/local-qa-runtime/workers/test/protocol-worker.test.mjs)
actual=$(find apps/local-qa-runtime -type f \( -name '*.rs' -o -name '*.ts' -o -name '*.mjs' \) \
  ! -path '*/node_modules/*' ! -path '*/target/*' ! -path '*/dist/*' | sort)
[[ "$actual" == "$expected" ]] || { echo 'unexpected Local QA source file' >&2; exit 1; }

for required in \
  apps/local-qa-runtime/host/Cargo.toml \
  apps/local-qa-runtime/host/src/coordinator.rs \
  apps/local-qa-runtime/host/src/executor.rs \
  apps/local-qa-runtime/host/src/journal.rs \
  apps/local-qa-runtime/host/src/lib.rs \
  apps/local-qa-runtime/host/src/main.rs \
  apps/local-qa-runtime/host/tests/fail_closed.rs \
  apps/local-qa-runtime/host/tests/loopback_sqlite.rs; do
  [[ -f "$required" ]] || { echo "missing Local QA Host file: $required" >&2; exit 1; }
done

grep -Eq '^[[:space:]]*"host",[[:space:]]*$' apps/local-qa-runtime/Cargo.toml || {
  echo 'Local QA Host is not registered in the Cargo workspace' >&2
  exit 1
}

grep -Eq '^[[:space:]]*"browser-adapter",[[:space:]]*$' apps/local-qa-runtime/Cargo.toml || {
  echo 'Local QA Browser adapter is not registered in the Cargo workspace' >&2
  exit 1
}

grep -Eq '^[[:space:]]*"evidence-stager",[[:space:]]*$' apps/local-qa-runtime/Cargo.toml || {
  echo 'Local QA Evidence stager is not registered in the Cargo workspace' >&2
  exit 1
}

evidence_stager_manifest=apps/local-qa-runtime/evidence-stager/Cargo.toml
grep -Eq '^name = "fkst-local-qa-evidence-stager"$' "$evidence_stager_manifest" || {
  echo 'Local QA Evidence stager package name is incorrect' >&2
  exit 1
}
grep -Eq '^fkst-qa-contracts = \{ path = "\.\./\.\./\.\./packages/qa-contracts/rust" \}$' "$evidence_stager_manifest" || {
  echo 'Local QA Evidence stager must consume the checked-in QA contracts API' >&2
  exit 1
}

browser_adapter_manifest=apps/local-qa-runtime/browser-adapter/Cargo.toml
grep -Eq '^name = "fkst-local-qa-browser-adapter"$' "$browser_adapter_manifest" || {
  echo 'Local QA Browser adapter package name is incorrect' >&2
  exit 1
}
grep -Eq '^headless_chrome = \{ version = "=1\.0\.22", default-features = false, features = \["offline"\] \}$' "$browser_adapter_manifest" || {
  echo 'Local QA Browser adapter must use the pinned Chrome automation dependency' >&2
  exit 1
}

host_manifest=apps/local-qa-runtime/host/Cargo.toml
[[ $(grep -Ec '^name = "fkst-local-qa-host"$' "$host_manifest") -eq 2 ]] || {
  echo 'Local QA Host package and binary names must match' >&2
  exit 1
}
grep -Eq '^ctrlc = \{ version = "=3\.4\.7", features = \["termination"\] \}$' "$host_manifest" || {
  echo 'Local QA Host must use the pinned termination-signal dependency' >&2
  exit 1
}
grep -Eq '^fkst-qa-contracts = \{ path = "\.\./\.\./\.\./packages/qa-contracts/rust" \}$' "$host_manifest" || {
  echo 'Local QA Host must consume the checked-in QA contracts API' >&2
  exit 1
}
grep -Eq '^rusqlite = "=0\.40\.1"$' "$host_manifest" || {
  echo 'Local QA Host must use the pinned SQLite dependency' >&2
  exit 1
}
grep -Eq '^serde = \{ version = "=1\.0\.229", features = \["derive"\] \}$' "$host_manifest" || {
  echo 'Local QA Host must use the pinned serde dependency' >&2
  exit 1
}
grep -Eq '^serde_json = "=1\.0\.151"$' "$host_manifest" || {
  echo 'Local QA Host must use the pinned JSON dependency' >&2
  exit 1
}
host_sources=(apps/local-qa-runtime/host/src/*.rs)
! grep -Eq '(unsafe|extern crate|include_(bytes|str)!)' "${host_sources[@]}" || {
  echo 'Local QA Host gained unauthorized production inclusion' >&2
  exit 1
}
removed_executor_artifacts=(
  'trait Executor'
  'impl Executor for'
  'LegacyExecutorAdapter'
  'Box<dyn Executor>'
  'legacy_executor_descriptor'
  'legacy_executor_selection'
  'legacy.executor'
  'legacy.execute'
  'sha256:e4760210c40c509504bf4cbf529835fc895e1b7d8e6cc3313fa673658e56a787'
  'PassingExecutor'
  'CoordinatorHandle::start('
)
for removed_artifact in "${removed_executor_artifacts[@]}"; do
  ! grep -Fq "$removed_artifact" "${host_sources[@]}" || {
    echo "Local QA Host restored removed Executor artifact: $removed_artifact" >&2
    exit 1
  }
done
grep -Fq '"/v1/runs/"' apps/local-qa-runtime/host/src/lib.rs
grep -Fq 'CREATE TABLE accepted_requests' apps/local-qa-runtime/host/src/journal.rs
grep -Fq 'CREATE TABLE runs' apps/local-qa-runtime/host/src/journal.rs
grep -Fq 'CREATE TABLE events' apps/local-qa-runtime/host/src/journal.rs
grep -Fq 'CREATE TABLE execution_attempts' apps/local-qa-runtime/host/src/journal.rs
grep -Fq 'PRAGMA user_version = 3' apps/local-qa-runtime/host/src/journal.rs
grep -Fq '"journal_mode", "WAL"' apps/local-qa-runtime/host/src/journal.rs
grep -Fq 'validate_local_state' apps/local-qa-runtime/host/src/journal.rs
grep -Fq 'validate_execution_outcome' apps/local-qa-runtime/host/src/journal.rs
grep -Fq 'CoordinatorHandle::start' apps/local-qa-runtime/host/src/lib.rs

for component in guest-agent launcher secret-broker supervisor; do
  manifest="apps/local-qa-runtime/$component/Cargo.toml"
  source="apps/local-qa-runtime/$component/src/main.rs"
  ! grep -Eq '^\[(build-dependencies|dependencies|dev-dependencies|features)\]' "$manifest"
  code=$(grep -Ev '^//!|^$' "$source")
  [[ "$code" == 'fn main() {}' ]]
done

node - <<'NODE'
const pkg = require('./apps/local-qa-runtime/workers/package.json');
const dependencies = { '@chronoai/fkst-qa-contracts': 'file:../../../packages/qa-contracts' };
const devDependencies = { '@types/node': '20.19.43', typescript: '5.9.3' };
const bin = { 'fkst-local-qa-worker': 'dist/worker-main.js' };
const expectedScripts = {
  build: 'npm run build --prefix ../../../packages/qa-contracts && tsc -p tsconfig.build.json',
  typecheck: 'npm run build --prefix ../../../packages/qa-contracts && tsc -p tsconfig.json',
  test: 'npm run build --prefix ../../../packages/qa-contracts && tsc -p tsconfig.build.json && node --test test/*.test.mjs',
};
if (JSON.stringify(pkg.dependencies) !== JSON.stringify(dependencies) ||
    JSON.stringify(pkg.scripts) !== JSON.stringify(expectedScripts) ||
    JSON.stringify(pkg.devDependencies) !== JSON.stringify(devDependencies) ||
    JSON.stringify(pkg.bin) !== JSON.stringify(bin)) {
  throw new Error('workers package gained unreviewed dependencies, scripts, or executables');
}
NODE

worker_sources=(apps/local-qa-runtime/workers/src/*.ts)
! grep -Eq "(playwright|node:(child_process|crypto|fs|http|https|net|os|path)|from[[:space:]]+['\"](child_process|crypto|fs|http|https|net|os|path)['\"]|process\.env|Deno\.|Bun\.)" "${worker_sources[@]}" || {
  echo 'workers gained a forbidden effect or hashing capability' >&2
  exit 1
}
grep -Fq 'runBrowserSmoke' apps/local-qa-runtime/workers/src/policy.ts
grep -Fq 'stageGeneratedLog' apps/local-qa-runtime/workers/src/policy.ts
grep -Fq 'session.close()' apps/local-qa-runtime/workers/src/policy.ts
grep -Fq 'request.duplicate_key' apps/local-qa-runtime/workers/src/json.ts

unexpected=$(find apps/local-qa-runtime -type f \
  \( -perm -111 -o -name '*.sh' -o -name '*.py' -o -name '*.js' -o -name '*.mjs' \) \
  ! -path 'apps/local-qa-runtime/tests/scaffold-structure.sh' \
  ! -path 'apps/local-qa-runtime/workers/test/browser-smoke.test.mjs' \
  ! -path 'apps/local-qa-runtime/workers/test/protocol-worker.test.mjs' \
  ! -path '*/node_modules/*' ! -path '*/target/*' ! -path '*/dist/*')
[[ -z "$unexpected" ]] || { echo 'unexpected executable implementation file' >&2; exit 1; }

echo 'Local QA Host coordinator, Evidence stager, pure worker policy, and inert Runtime shells are complete.'
