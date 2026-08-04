#!/usr/bin/env bash
set -euo pipefail

root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)
cd "$root"

expected=$(printf '%s\n' \
  apps/local-qa-runtime/guest-agent/src/main.rs \
  apps/local-qa-runtime/host/src/lib.rs \
  apps/local-qa-runtime/host/src/main.rs \
  apps/local-qa-runtime/host/tests/fail_closed.rs \
  apps/local-qa-runtime/host/tests/loopback_sqlite.rs \
  apps/local-qa-runtime/launcher/src/main.rs \
  apps/local-qa-runtime/secret-broker/src/main.rs \
  apps/local-qa-runtime/supervisor/src/main.rs \
  apps/local-qa-runtime/workers/src/index.ts)
actual=$(find apps/local-qa-runtime -type f \( -name '*.rs' -o -name '*.ts' \) \
  ! -path '*/node_modules/*' ! -path '*/target/*' ! -path '*/dist/*' | sort)
[[ "$actual" == "$expected" ]] || { echo 'unexpected Local QA source file' >&2; exit 1; }

for required in \
  apps/local-qa-runtime/host/Cargo.toml \
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

host_manifest=apps/local-qa-runtime/host/Cargo.toml
[[ $(grep -Ec '^name = "fkst-local-qa-host"$' "$host_manifest") -eq 2 ]] || {
  echo 'Local QA Host package and binary names must match' >&2
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
host_sources=(apps/local-qa-runtime/host/src/lib.rs apps/local-qa-runtime/host/src/main.rs)
! grep -Eq '(unsafe|extern crate|include_(bytes|str)!)' "${host_sources[@]}" || {
  echo 'Local QA Host gained unauthorized production inclusion' >&2
  exit 1
}
grep -Fq '"/v1/runs/"' apps/local-qa-runtime/host/src/lib.rs
grep -Fq 'CREATE TABLE accepted_requests' apps/local-qa-runtime/host/src/lib.rs
grep -Fq 'CREATE TABLE runs' apps/local-qa-runtime/host/src/lib.rs
grep -Fq 'CREATE TABLE events' apps/local-qa-runtime/host/src/lib.rs
grep -Fq '"journal_mode", "WAL"' apps/local-qa-runtime/host/src/lib.rs

for component in guest-agent launcher secret-broker supervisor; do
  manifest="apps/local-qa-runtime/$component/Cargo.toml"
  source="apps/local-qa-runtime/$component/src/main.rs"
  ! grep -Eq '^\[(build-dependencies|dependencies|dev-dependencies|features)\]' "$manifest"
  code=$(grep -Ev '^//!|^$' "$source")
  [[ "$code" == 'fn main() {}' ]]
done

node - <<'NODE'
const pkg = require('./apps/local-qa-runtime/workers/package.json');
const scripts = { build: 'tsc -p tsconfig.build.json', typecheck: 'tsc -p tsconfig.json' };
if (pkg.dependencies || JSON.stringify(pkg.scripts) !== JSON.stringify(scripts) ||
    JSON.stringify(pkg.devDependencies) !== JSON.stringify({ typescript: '5.9.3' })) {
  throw new Error('workers scaffold gained runtime behavior or dependencies');
}
NODE

worker_code=$(grep -Ev '^//|^$' apps/local-qa-runtime/workers/src/index.ts)
[[ "$worker_code" == 'export {};' ]]

unexpected=$(find apps/local-qa-runtime -type f \
  \( -perm -111 -o -name '*.sh' -o -name '*.py' -o -name '*.js' -o -name '*.mjs' \) \
  ! -path 'apps/local-qa-runtime/tests/scaffold-structure.sh' \
  ! -path '*/node_modules/*' ! -path '*/target/*' ! -path '*/dist/*')
[[ -z "$unexpected" ]] || { echo 'unexpected executable implementation file' >&2; exit 1; }

echo 'Local QA Host boundary and inert Runtime shells are complete.'
