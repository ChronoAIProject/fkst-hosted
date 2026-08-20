#!/usr/bin/env bash

set -euo pipefail

usage() {
  printf '%s\n' 'Usage: scripts/run.sh test | scripts/run.sh test <backend|frontend|local-qa-runtime|qa-contracts> | scripts/run.sh test-affected' >&2
}

if [[ "$#" -ne 2 || "$1" != 'test' || ( "$2" != 'backend' && "$2" != 'frontend' ) ]]; then
  usage
  exit 1
fi

script_dir=$(CDPATH= cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)
repository_root=$(CDPATH= cd -- "$script_dir/.." && pwd -P)

if [[ "$2" == 'frontend' ]]; then
  cd "$repository_root/frontend"

  npm ci
  npm run lint
  npm run typecheck
  npm run test
  npm run build
  exit 0
fi

cd "$repository_root/backend"

cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo build --workspace --locked
cargo test --workspace --locked
