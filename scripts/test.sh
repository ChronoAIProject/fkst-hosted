#!/usr/bin/env bash
# Run package unit + integration tests via `fkst-framework test` (test mode:
# *_test.lua + fkst.test.run_department). Mirrors what CI runs.
#
# Usage:
#   scripts/test.sh                # all packages that have tests
#   scripts/test.sh github-proxy   # one package
#   BIN=/path/to/fkst-framework scripts/test.sh   # explicit binary
set -euo pipefail

DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$DIR/.." && pwd)"
# shellcheck source=lib.sh
. "$DIR/lib.sh"
resolve_bin
echo "BIN=$BIN"

target="${1:-}"
ran=0
fail=0
for pkg in "$ROOT"/packages/*/; do
  name="$(basename "$pkg")"
  if [ -n "$target" ] && [ "$name" != "$target" ]; then
    continue
  fi
  if ! ls "$pkg"tests/*_test.lua >/dev/null 2>&1; then
    continue
  fi
  echo "=== $name ==="
  ran=$((ran + 1))
  if ! "$BIN" test --project-root "$pkg" --package-root "$pkg"; then
    fail=$((fail + 1))
  fi
done

if [ "$ran" -eq 0 ]; then
  echo "no packages with tests matched${target:+ for '$target'}" >&2
  exit 1
fi
if [ "$fail" -ne 0 ]; then
  echo "FAILED: $fail/$ran package(s)" >&2
  exit 1
fi
echo "OK: $ran package(s) passed"
