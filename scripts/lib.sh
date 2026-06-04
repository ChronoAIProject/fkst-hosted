# Shared helpers for package scripts. Source this; it only defines functions.

repo_root() {
  cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd
}

# Resolve the fkst-framework binary into $BIN, in priority order:
#   1. existing $BIN env override
#   2. BIN= in repo-root .env (gitignored local config; see env.example)
#   3. fkst-framework on PATH
#   4. sibling checkout ../fkst-substrate/target/debug/fkst-framework
# Exits non-zero with a hint if none is found / executable.
resolve_bin() {
  local root
  root="$(repo_root)"
  if [ -z "${BIN:-}" ] && [ -f "$root/.env" ]; then
    BIN="$(grep -E '^BIN=' "$root/.env" | tail -1 | cut -d= -f2-)"
  fi
  if [ -z "${BIN:-}" ]; then
    if command -v fkst-framework >/dev/null 2>&1; then
      BIN="$(command -v fkst-framework)"
    elif [ -x "$root/../fkst-substrate/target/debug/fkst-framework" ]; then
      BIN="$root/../fkst-substrate/target/debug/fkst-framework"
    fi
  fi
  if [ -z "${BIN:-}" ] || [ ! -x "$BIN" ]; then
    echo "error: fkst-framework binary not found (looked at \$BIN, .env, PATH, ../fkst-substrate)." >&2
    echo "  fix: cp env.example .env  (set BIN=), or build the engine:" >&2
    echo "       (cd ../fkst-substrate && git pull && cargo build -p fkst-framework)" >&2
    exit 1
  fi
  export BIN
}
