#!/usr/bin/env bash
# Read-only preflight checks for the local fkst package development environment.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

# shellcheck source=scripts/bin_bootstrap.sh
. "$ROOT/scripts/bin_bootstrap.sh"

DOCTOR_FAIL=0
DOCTOR_RESOLVED_BIN=""

doctor_emit() {
  local item="$1" state="$2" detail="${3:-}"
  if [ -n "$detail" ]; then
    printf 'DOCTOR %s %s %s\n' "$item" "$state" "$detail"
  else
    printf 'DOCTOR %s %s\n' "$item" "$state"
  fi
}

doctor_missing() {
  doctor_emit "$1" "missing" "$2"
  DOCTOR_FAIL=1
}

doctor_ok() {
  doctor_emit "$1" "ok" "$2"
}

doctor_first_line() {
  printf '%s\n' "$1" | sed -n '1p'
}

doctor_check_tool() {
  local tool="$1" version_args="$2" hint="$3" path output version
  if ! path="$(command -v "$tool" 2>/dev/null)"; then
    doctor_missing "$tool" "hint=$hint"
    return 0
  fi
  if ! output="$("$tool" $version_args 2>&1)"; then
    version="$(doctor_first_line "$output")"
    doctor_missing "$tool" "hint=$hint detail=${version:-version-check-failed}"
    return 0
  fi
  version="$(doctor_first_line "$output")"
  doctor_ok "$tool" "path=$path version=${version:-unknown}"
}

doctor_resolve_bin_readonly() {
  local candidate=""
  DOCTOR_RESOLVED_BIN=""

  if [ -n "${BIN:-}" ]; then
    if [ -x "$BIN" ]; then
      DOCTOR_RESOLVED_BIN="$BIN"
      return 0
    fi
    doctor_missing "bin" "hint=explicit BIN is not executable: $BIN"
    return 1
  fi

  if [ -f "$ROOT/.env" ]; then
    candidate="$(grep -E '^BIN=' "$ROOT/.env" 2>/dev/null | tail -1 | cut -d= -f2- || true)"
    candidate="${candidate%%[[:space:]]#*}"
    candidate="${candidate%\"}"; candidate="${candidate#\"}"; candidate="${candidate%\'}"; candidate="${candidate#\'}"
    if [ -n "$candidate" ]; then
      if [ -x "$candidate" ]; then
        DOCTOR_RESOLVED_BIN="$candidate"
        return 0
      fi
      doctor_missing "bin" "hint=.env BIN is not executable: $candidate"
      return 1
    fi
  fi

  if candidate="$(command -v fkst-framework 2>/dev/null)"; then
    DOCTOR_RESOLVED_BIN="$candidate"
    return 0
  fi

  candidate="$ROOT/../fkst-substrate/target/debug/fkst-framework"
  if [ -x "$candidate" ]; then
    DOCTOR_RESOLVED_BIN="$candidate"
    return 0
  fi

  if [ -z "${FKST_NO_AUTOBUILD:-}" ] && [ -f "$ROOT/.fkst-substrate-ref" ]; then
    local pin owner repo ref cache_root cache_bin
    pin="$(bootstrap_read_pin "$ROOT" 2>/dev/null || true)"
    if [ -n "$pin" ]; then
      {
        IFS= read -r owner
        IFS= read -r repo
        IFS= read -r ref
      } < <(bootstrap_parse_pin "$pin" 2>/dev/null || true)
      cache_root="$(bootstrap_cache_root 2>/dev/null || true)"
      if [ -n "${owner:-}" ] && [ -n "${repo:-}" ] && [ -n "${ref:-}" ] && [ -n "$cache_root" ]; then
        cache_bin="$(bootstrap_cache_bin_path "$ROOT" "$cache_root" "$owner" "$repo" "$ref" 2>/dev/null || true)"
        if [ -x "$cache_bin" ]; then
          DOCTOR_RESOLVED_BIN="$cache_bin"
          return 0
        fi
      fi
    fi
  fi

  doctor_missing "bin" "hint=set BIN to an executable fkst-framework, put fkst-framework on PATH, build ../fkst-substrate, or run scripts/run.sh build"
  return 1
}

doctor_check_bin() {
  local resolved
  doctor_resolve_bin_readonly || return 0
  resolved="$DOCTOR_RESOLVED_BIN"
  doctor_ok "bin" "path=$resolved"
  if "$resolved" --self-test >/dev/null 2>&1; then
    doctor_ok "bin-self-test" "path=$resolved"
  else
    doctor_missing "bin-self-test" "hint=$resolved --self-test"
  fi
}

doctor_check_codex() {
  local path
  if path="$(command -v codex 2>/dev/null)"; then
    doctor_ok "codex" "path=$path"
  else
    doctor_missing "codex" "hint=npm install -g @openai/codex"
  fi
}

doctor_check_gh() {
  local path output version
  if ! path="$(command -v gh 2>/dev/null)"; then
    doctor_missing "gh" "hint=install GitHub CLI, then run gh auth login"
    return 0
  fi
  if output="$(gh --version 2>&1)"; then
    version="$(doctor_first_line "$output")"
  else
    version="unknown"
  fi
  doctor_ok "gh" "path=$path version=$version"
  if gh auth status >/dev/null 2>&1; then
    doctor_ok "gh-auth" "status=authenticated"
  else
    doctor_missing "gh-auth" "hint=gh auth login"
  fi
}

doctor_check_env() {
  local name value state
  for name in \
    FKST_GITHUB_REPO \
    FKST_GITHUB_BOT_LOGIN \
    FKST_GITHUB_WRITE \
    FKST_RUNTIME_ROOT \
    FKST_DURABLE_ROOT \
    FKST_RATE_POOL_ROOT \
    FKST_RATE_POOL_GH \
    FKST_PROJECT_ROOT \
    FKST_NO_AUTOBUILD \
    FKST_BIN_CACHE_ROOT; do
    value="${!name:-}"
    if [ -n "$value" ]; then
      state="ok"
      doctor_emit "env.$name" "$state" "value=$value"
    else
      doctor_emit "env.$name" "missing" "hint=optional host fact is unset"
    fi
  done
}

doctor_main() {
  if [ "$#" -ne 0 ]; then
    echo "usage: scripts/run.sh doctor" >&2
    return 2
  fi

  doctor_check_tool "git" "--version" "install git"
  doctor_check_tool "cargo" "--version" "install Rust with rustup"
  doctor_check_tool "rustc" "--version" "install Rust with rustup"
  doctor_check_bin
  doctor_check_codex
  doctor_check_gh
  doctor_check_env

  return "$DOCTOR_FAIL"
}

doctor_main "$@"
