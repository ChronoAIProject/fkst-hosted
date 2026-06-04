#!/usr/bin/env bash
# Generic integration runner for fkst packages.
#
#   scripts/integration.sh test [package]
#       Run *_test.lua (test mode: unit + fkst.test.run_department) for every
#       package, or just one. Mirrors what CI runs. Exits non-zero on failure.
#
#   scripts/integration.sh run <package> <department> [event-json]
#       One-shot run a department against the REAL host environment via
#       `fkst-framework run`: decode emitted RAISED events and dump the <RT>
#       scratch tree. Generic across packages; pass package-specific config via
#       env, e.g.:
#         FKST_GITHUB_REPO=owner/repo scripts/integration.sh run github-proxy github_poll
#       Reuses $FKST_RUNTIME_ROOT if already set (run twice with the same one to
#       observe dedup), else uses a fresh temp dir. Never sets FKST_GITHUB_WRITE,
#       so a read-only inbound dogfood stays read-only.
#
# fkst-framework binary resolution (priority): $BIN > repo .env `BIN=` > PATH >
# sibling ../fkst-substrate/target/debug/fkst-framework.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

resolve_bin() {
  if [ -z "${BIN:-}" ] && [ -f "$ROOT/.env" ]; then
    BIN="$(grep -E '^BIN=' "$ROOT/.env" | tail -1 | cut -d= -f2-)"
  fi
  if [ -z "${BIN:-}" ]; then
    if command -v fkst-framework >/dev/null 2>&1; then
      BIN="$(command -v fkst-framework)"
    elif [ -x "$ROOT/../fkst-substrate/target/debug/fkst-framework" ]; then
      BIN="$ROOT/../fkst-substrate/target/debug/fkst-framework"
    fi
  fi
  if [ -z "${BIN:-}" ] || [ ! -x "$BIN" ]; then
    echo "error: fkst-framework binary not found (\$BIN, .env, PATH, ../fkst-substrate)." >&2
    echo "  fix: cp env.example .env (set BIN=), or build the engine:" >&2
    echo "       (cd ../fkst-substrate && git pull && cargo build -p fkst-framework)" >&2
    exit 1
  fi
  export BIN
}

usage() {
  sed -n '2,20p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'
}

cmd_test() {
  local target="${1:-}" ran=0 fail=0 pkg name
  for pkg in "$ROOT"/packages/*/; do
    name="$(basename "$pkg")"
    if [ -n "$target" ] && [ "$name" != "$target" ]; then continue; fi
    if ! ls "$pkg"tests/*_test.lua >/dev/null 2>&1; then continue; fi
    echo "=== $name ==="
    ran=$((ran + 1))
    if ! "$BIN" test --project-root "$pkg" --package-root "$pkg"; then
      fail=$((fail + 1))
    fi
  done
  if [ "$ran" -eq 0 ]; then
    echo "no packages with tests matched${target:+ for '$target'}" >&2; exit 1
  fi
  if [ "$fail" -ne 0 ]; then
    echo "FAILED: $fail/$ran package(s)" >&2; exit 1
  fi
  echo "OK: $ran package(s) passed"
}

cmd_run() {
  local pkg="${1:-}" dept="${2:-}" event="${3:-{\"payload\":{}}}"
  if [ -z "$pkg" ] || [ -z "$dept" ]; then
    echo "usage: scripts/integration.sh run <package> <department> [event-json]" >&2; exit 1
  fi
  local pkgdir="$ROOT/packages/$pkg" lua
  lua="$pkgdir/departments/$dept/main.lua"
  [ -f "$lua" ] || { echo "error: no department at $lua" >&2; exit 1; }

  local rt fresh=0
  if [ -n "${FKST_RUNTIME_ROOT:-}" ]; then
    rt="$FKST_RUNTIME_ROOT"
  else
    rt="$(mktemp -d "${TMPDIR:-/tmp}/fkst-integration.XXXXXX")"; fresh=1
  fi
  export FKST_RUNTIME_ROOT="$rt"

  echo "BIN=$BIN"
  echo "run $pkg/$dept  FKST_RUNTIME_ROOT=$rt${fresh:+ (fresh)}"
  if [ -n "${FKST_GITHUB_REPO:-}" ]; then echo "FKST_GITHUB_REPO=$FKST_GITHUB_REPO"; fi

  local out
  out="$("$BIN" run "$lua" --project-root "$ROOT" --package-root "$pkgdir" --event "$event" 2>&1)"

  echo "--- logs ---"
  printf '%s\n' "$out" | grep -vE '^RAISED:' || true
  echo "--- raised events (decoded) ---"
  local b64
  b64="$(printf '%s\n' "$out" | grep '^RAISED:' | sed 's/^RAISED: //' | tail -1 || true)"
  if [ -n "$b64" ]; then
    printf '%s' "$b64" | base64 -d 2>/dev/null | python3 -m json.tool 2>/dev/null \
      || { echo "(raw)"; printf '%s' "$b64" | base64 -d 2>/dev/null; }
  else
    echo "  (no events raised)"
  fi
  echo "--- <RT> tree ---"
  find "$rt" -type f 2>/dev/null | sort | while read -r f; do
    echo "  ${f#"$rt"/} = $(cat "$f" 2>/dev/null | head -c 120)"
  done
}

case "${1:-}" in
  test) shift; resolve_bin; cmd_test "$@" ;;
  run)  shift; resolve_bin; cmd_run "$@" ;;
  -h|--help|help|"") usage ;;
  *) echo "unknown subcommand: $1" >&2; usage; exit 1 ;;
esac
