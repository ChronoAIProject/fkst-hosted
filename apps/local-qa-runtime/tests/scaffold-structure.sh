#!/usr/bin/env bash
set -euo pipefail

root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)
cd "$root"

expected=$(printf '%s\n' \
  apps/local-qa-runtime/guest-agent/src/main.rs \
  apps/local-qa-runtime/launcher/src/main.rs \
  apps/local-qa-runtime/secret-broker/src/main.rs \
  apps/local-qa-runtime/supervisor/src/main.rs \
  apps/local-qa-runtime/workers/src/index.ts)
actual=$(find apps/local-qa-runtime -type f \( -name '*.rs' -o -name '*.ts' \) \
  ! -path '*/node_modules/*' ! -path '*/target/*' ! -path '*/dist/*' | sort)
[[ "$actual" == "$expected" ]] || { echo 'unexpected Local QA source file' >&2; exit 1; }

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

echo 'Local QA Runtime scaffold structure is inert and complete.'
