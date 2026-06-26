#!/usr/bin/env bash
# Host-run contract helpers for scripts/run.sh supervise.

HOST_RUN_PROJECT_ROOT=""
HOST_RUN_PLATFORM_ROOT=""
HOST_RUN_LOCAL_PACKAGES_ROOT=""
HOST_RUN_PLATFORM_PACKAGES=""
HOST_RUN_HOST_PACKAGES=""
HOST_RUN_DURABLE_ROOT=""
HOST_RUN_RUNTIME_ROOT=""
HOST_RUN_RUNTIME_BASE=""
HOST_RUN_RUNTIME_LABEL=""
HOST_RUN_RUNTIME_IS_EXPLICIT=0
HOST_RUN_RESTART=0
HOST_RUN_PACKAGE_ROOTS=()
HOST_RUN_PLATFORM_EXTERNAL_SOURCE_ID="fkst-packages-platform"

host_run_usage() {
  cat >&2 <<'EOF'
usage: scripts/run.sh supervise --project-root <HOST> --platform-root <PKGSRC> --platform-packages "<names>" [--host-packages "<names>"] --durable-root <path> [--runtime-root <fresh-scratch-root>] [--restart]
   or: scripts/run.sh supervise <package>
EOF
}

host_run_abs_path() {
  local path="$1"
  case "$path" in
    /*) printf '%s\n' "$path" ;;
    *) printf '%s/%s\n' "$(pwd -P)" "$path" ;;
  esac
}

host_run_same_path() {
  local left="$1" right="$2" left_phys right_phys
  left_phys="$(cd "$left" 2>/dev/null && pwd -P)" || return 1
  right_phys="$(cd "$right" 2>/dev/null && pwd -P)" || return 1
  [ "$left_phys" = "$right_phys" ]
}

host_run_platform_source_spec() {
  local workspace_file="$HOST_RUN_PROJECT_ROOT/fkst.workspace.toml" lock_file="$HOST_RUN_PROJECT_ROOT/fkst.lock"
  [ -f "$workspace_file" ] || [ -f "$lock_file" ] || return 0
  python3 - "$workspace_file" "$lock_file" "$HOST_RUN_PLATFORM_EXTERNAL_SOURCE_ID" <<'PY'
from __future__ import annotations

from pathlib import Path
import sys

try:
    import tomllib
except ModuleNotFoundError:
    print("error: python3 tomllib is required to read host external source pins", file=sys.stderr)
    raise SystemExit(1)


workspace_file = Path(sys.argv[1])
lock_file = Path(sys.argv[2])
source_id = sys.argv[3]


def load_toml(path: Path) -> dict:
    if not path.exists():
        return {}
    with path.open("rb") as handle:
        return tomllib.load(handle)


def find_source(items: object, id_value: str) -> dict | None:
    if not isinstance(items, list):
        return None
    matches = [item for item in items if isinstance(item, dict) and item.get("id") == id_value]
    if len(matches) > 1:
        print(f"error: duplicate external source id {id_value!r}", file=sys.stderr)
        raise SystemExit(1)
    return matches[0] if matches else None


workspace = load_toml(workspace_file)
lock = load_toml(lock_file)
workspace_source = find_source(workspace.get("external_sources"), source_id)
lock_source = find_source(lock.get("external_source"), source_id)

if workspace_source is None and lock_source is None:
    raise SystemExit(0)
if workspace_source is None:
    print(f"error: {lock_file} declares {source_id} but {workspace_file} does not", file=sys.stderr)
    raise SystemExit(1)
if lock_source is None:
    print(f"error: {workspace_file} declares {source_id} but {lock_file} does not", file=sys.stderr)
    raise SystemExit(1)

workspace_git = workspace_source.get("git")
lock_git = lock_source.get("git")
if not isinstance(workspace_git, str) or workspace_git == "":
    print(f"error: {workspace_file} external source {source_id} has no git URL", file=sys.stderr)
    raise SystemExit(1)
if workspace_git != lock_git:
    print(f"error: {source_id} git URL mismatch between workspace and lock", file=sys.stderr)
    raise SystemExit(1)

workspace_rev = workspace_source.get("rev")
intent = lock_source.get("intent")
intent_rev = intent.get("rev") if isinstance(intent, dict) else None
if isinstance(workspace_rev, str) and isinstance(intent_rev, str) and workspace_rev != intent_rev:
    print(f"error: {source_id} intent rev mismatch between workspace and lock", file=sys.stderr)
    raise SystemExit(1)

resolved = lock_source.get("resolved")
resolved_rev = resolved.get("rev") if isinstance(resolved, dict) else None
if not isinstance(resolved_rev, str) or resolved_rev == "":
    print(f"error: {lock_file} external source {source_id} has no resolved rev", file=sys.stderr)
    raise SystemExit(1)

print(source_id)
print(workspace_git)
print(resolved_rev)
PY
}

host_run_dir_is_empty() {
  local path="$1"
  [ -d "$path" ] || return 0
  [ -z "$(find "$path" -mindepth 1 -maxdepth 1 2>/dev/null | head -1)" ]
}

host_run_checkout_ref() {
  local checkout_dir="$1" ref="$2"
  if git -C "$checkout_dir" checkout --detach "$ref" 1>&2; then
    return 0
  fi
  git -C "$checkout_dir" checkout --detach "origin/$ref" 1>&2
}

host_run_hydrate_platform_source() {
  local spec source_id git_url resolved_rev target head
  spec="$(host_run_platform_source_spec)" || return $?
  [ -n "$spec" ] || return 0

  source_id="$(printf '%s\n' "$spec" | sed -n '1p')"
  git_url="$(printf '%s\n' "$spec" | sed -n '2p')"
  resolved_rev="$(printf '%s\n' "$spec" | sed -n '3p')"
  target="$HOST_RUN_PROJECT_ROOT/.fkst/run/$source_id"

  mkdir -p "$(dirname "$target")"
  if [ -d "$target/.git" ]; then
    git -C "$target" remote set-url origin "$git_url" 1>&2
    git -C "$target" fetch --tags origin '+refs/heads/*:refs/remotes/origin/*' 1>&2 || return $?
  else
    if ! host_run_dir_is_empty "$target"; then
      echo "error: platform hydration target is not an empty directory or git checkout: $target" >&2
      return 1
    fi
    git clone --no-checkout "$git_url" "$target" 1>&2 || return $?
    git -C "$target" fetch --tags origin '+refs/heads/*:refs/remotes/origin/*' 1>&2 || return $?
  fi

  host_run_checkout_ref "$target" "$resolved_rev" || return $?
  head="$(git -C "$target" rev-parse HEAD 2>/dev/null)" || return $?
  if [ "$head" != "$resolved_rev" ]; then
    echo "error: hydrated $source_id at $target resolved to $head, expected $resolved_rev" >&2
    return 1
  fi
  HOST_RUN_PLATFORM_ROOT="$target"
}

host_run_parse_supervise_args() {
  HOST_RUN_PROJECT_ROOT=""
  HOST_RUN_PLATFORM_ROOT=""
  HOST_RUN_LOCAL_PACKAGES_ROOT=""
  HOST_RUN_PLATFORM_PACKAGES=""
  HOST_RUN_HOST_PACKAGES=""
  HOST_RUN_DURABLE_ROOT=""
  HOST_RUN_RUNTIME_ROOT=""
  HOST_RUN_RUNTIME_BASE=""
  HOST_RUN_RUNTIME_LABEL=""
  HOST_RUN_RUNTIME_IS_EXPLICIT=0
  HOST_RUN_RESTART=0

  while [ "$#" -gt 0 ]; do
    case "$1" in
      --project-root)
        [ "$#" -ge 2 ] || { echo "error: --project-root requires a path" >&2; return 2; }
        HOST_RUN_PROJECT_ROOT="$2"; shift 2 ;;
      --platform-root)
        [ "$#" -ge 2 ] || { echo "error: --platform-root requires a path" >&2; return 2; }
        HOST_RUN_PLATFORM_ROOT="$2"; shift 2 ;;
      --local-packages)
        [ "$#" -ge 2 ] || { echo "error: --local-packages requires a path" >&2; return 2; }
        HOST_RUN_LOCAL_PACKAGES_ROOT="$2"; shift 2 ;;
      --platform-packages)
        [ "$#" -ge 2 ] || { echo "error: --platform-packages requires a package list" >&2; return 2; }
        HOST_RUN_PLATFORM_PACKAGES="$2"; shift 2 ;;
      --host-packages)
        [ "$#" -ge 2 ] || { echo "error: --host-packages requires a package list" >&2; return 2; }
        HOST_RUN_HOST_PACKAGES="$2"; shift 2 ;;
      --durable-root)
        [ "$#" -ge 2 ] || { echo "error: --durable-root requires a path" >&2; return 2; }
        HOST_RUN_DURABLE_ROOT="$2"; shift 2 ;;
      --runtime-root)
        [ "$#" -ge 2 ] || { echo "error: --runtime-root requires a path" >&2; return 2; }
        HOST_RUN_RUNTIME_BASE="$2"; HOST_RUN_RUNTIME_IS_EXPLICIT=1; shift 2 ;;
      --restart)
        HOST_RUN_RESTART=1; shift ;;
      -h|--help)
        host_run_usage; return 2 ;;
      *)
        echo "error: unknown supervise option: $1" >&2
        host_run_usage
        return 2 ;;
    esac
  done

  [ -n "$HOST_RUN_PROJECT_ROOT" ] || { echo "error: --project-root is required" >&2; return 2; }
  [ -n "$HOST_RUN_PLATFORM_ROOT" ] || { echo "error: --platform-root is required" >&2; return 2; }
  [ -n "$HOST_RUN_PLATFORM_PACKAGES" ] || { echo "error: --platform-packages is required" >&2; return 2; }
  [ -n "$HOST_RUN_DURABLE_ROOT" ] || { echo "error: --durable-root is required for explicit supervise" >&2; return 2; }

  HOST_RUN_PROJECT_ROOT="$(host_run_abs_path "$HOST_RUN_PROJECT_ROOT")"
  HOST_RUN_PLATFORM_ROOT="$(host_run_abs_path "$HOST_RUN_PLATFORM_ROOT")"
  if [ -n "$HOST_RUN_LOCAL_PACKAGES_ROOT" ]; then
    HOST_RUN_LOCAL_PACKAGES_ROOT="$(host_run_abs_path "$HOST_RUN_LOCAL_PACKAGES_ROOT")"
  fi
  HOST_RUN_DURABLE_ROOT="$(host_run_abs_path "$HOST_RUN_DURABLE_ROOT")"
  if [ -n "$HOST_RUN_RUNTIME_BASE" ]; then
    HOST_RUN_RUNTIME_BASE="$(host_run_abs_path "$HOST_RUN_RUNTIME_BASE")"
  else
    HOST_RUN_RUNTIME_ROOT="$(mktemp -d "${TMPDIR:-/tmp}/fkst-host-run-rt.XXXXXX")"
    HOST_RUN_RUNTIME_LABEL="fresh temp"
  fi
}

host_run_validate_shape() {
  [ -d "$HOST_RUN_PROJECT_ROOT" ] || { echo "error: project root does not exist: $HOST_RUN_PROJECT_ROOT" >&2; return 1; }
  [ -d "$HOST_RUN_PLATFORM_ROOT" ] || { echo "error: platform root does not exist: $HOST_RUN_PLATFORM_ROOT" >&2; return 1; }
  [ -d "$HOST_RUN_PLATFORM_ROOT/packages" ] || { echo "error: platform root has no packages directory: $HOST_RUN_PLATFORM_ROOT/packages" >&2; return 1; }
  mkdir -p "$HOST_RUN_DURABLE_ROOT"
  if [ "$HOST_RUN_RUNTIME_IS_EXPLICIT" -eq 1 ]; then
    mkdir -p "$HOST_RUN_RUNTIME_BASE"
    HOST_RUN_RUNTIME_ROOT="$HOST_RUN_RUNTIME_BASE"
    HOST_RUN_RUNTIME_LABEL="explicit"
    if host_run_same_path "$HOST_RUN_RUNTIME_ROOT" "$HOST_RUN_DURABLE_ROOT"; then
      echo "error: --runtime-root and --durable-root resolved to the same directory" >&2
      return 1
    fi
  fi
  if host_run_same_path "$HOST_RUN_RUNTIME_ROOT" "$HOST_RUN_DURABLE_ROOT"; then
    echo "error: --runtime-root and --durable-root resolved to the same directory" >&2
    return 1
  fi
}

host_run_host_package_base() {
  if host_run_same_path "$HOST_RUN_PROJECT_ROOT" "$HOST_RUN_PLATFORM_ROOT"; then
    printf '%s/packages\n' "$HOST_RUN_PROJECT_ROOT"
    return 0
  fi
  if [ -n "$HOST_RUN_LOCAL_PACKAGES_ROOT" ]; then
    printf '%s\n' "$HOST_RUN_LOCAL_PACKAGES_ROOT"
    return 0
  fi
  printf '%s/.fkst/local-packages\n' "$HOST_RUN_PROJECT_ROOT"
}

host_run_add_named_roots() {
  local base="$1" kind="$2" names="$3" name path
  for name in $names; do
    path="$base/$name"
    [ -d "$path" ] || { echo "error: missing $kind package '$name' at $path" >&2; return 1; }
    HOST_RUN_PACKAGE_ROOTS+=("$path")
  done
}

host_run_build_package_roots() {
  HOST_RUN_PACKAGE_ROOTS=()
  host_run_add_named_roots "$HOST_RUN_PLATFORM_ROOT/packages" "platform" "$HOST_RUN_PLATFORM_PACKAGES" || return 1
  if [ -n "$HOST_RUN_HOST_PACKAGES" ]; then
    host_run_add_named_roots "$(host_run_host_package_base)" "host" "$HOST_RUN_HOST_PACKAGES" || return 1
  fi
}

host_run_pid_file() {
  printf '%s/.fkst-supervise.pid\n' "$HOST_RUN_DURABLE_ROOT"
}

host_run_pid_check() {
  local pid="$1" err
  err="$(kill -0 "$pid" 2>&1)" && return 0
  case "$err" in
    *"Operation not permitted"*|*"operation not permitted"*|*"not permitted"*)
      return 2
      ;;
  esac
  return 1
}

host_run_pid_state() {
  local pid="$1" stat
  if [ -r "/proc/$pid/stat" ]; then
    stat="$(sed 's/^.*) //' "/proc/$pid/stat" 2>/dev/null | awk '{print $1}')" || stat=""
    [ -n "$stat" ] && { printf '%s\n' "$stat"; return 0; }
  fi
  stat="$(ps -o stat= -p "$pid" 2>/dev/null | awk 'NF {print $1; exit}')" || stat=""
  [ -n "$stat" ] && { printf '%s\n' "$stat"; return 0; }
  return 1
}

host_run_pid_is_dead() {
  local pid="$1" state
  host_run_pid_check "$pid"
  case "$?" in
    0) ;;
    1) return 0 ;;
    *) return 1 ;;
  esac
  state="$(host_run_pid_state "$pid" 2>/dev/null || true)"
  [[ "$state" == Z* ]]
}

host_run_kill_supervise_pid() {
  local pid="$1" pid_file="$2" attempts=0
  if host_run_pid_is_dead "$pid"; then
    echo "restart: removing stale supervise pidfile for dead pid $pid at $pid_file" >&2
    rm -f "$pid_file"
    return 0
  fi
  echo "restart: killing prior supervise pid $pid for durable root $HOST_RUN_DURABLE_ROOT" >&2
  if ! kill -9 "$pid" 2>/dev/null; then
    echo "error: failed to SIGKILL prior supervise pid $pid from $pid_file; refusing to launch a second supervise on $HOST_RUN_DURABLE_ROOT" >&2
    return 1
  fi
  while [ "$attempts" -lt 50 ]; do
    if host_run_pid_is_dead "$pid"; then
      rm -f "$pid_file"
      return 0
    fi
    attempts=$((attempts + 1))
    sleep 0.1
  done
  echo "error: prior supervise pid $pid from $pid_file is still alive after SIGKILL; refusing to launch a second supervise on $HOST_RUN_DURABLE_ROOT" >&2
  return 1
}

host_run_restart_prior() {
  local pid_file pid
  [ "$HOST_RUN_RESTART" -eq 1 ] || return 0
  pid_file="$(host_run_pid_file)"
  [ -f "$pid_file" ] || return 0
  pid="$(sed -n '1p' "$pid_file" 2>/dev/null || true)"
  case "$pid" in
    ''|*[!0-9]*)
      echo "error: malformed supervise pidfile at $pid_file; refusing to launch a second supervise on $HOST_RUN_DURABLE_ROOT" >&2
      return 1
      ;;
    *)
      host_run_kill_supervise_pid "$pid" "$pid_file"
      ;;
  esac
}

host_run_claim_supervise_slot() {
  local pid_file pid wrote=0
  pid_file="$(host_run_pid_file)"
  if [ -f "$pid_file" ]; then
    pid="$(sed -n '1p' "$pid_file" 2>/dev/null || true)"
    case "$pid" in
      ''|*[!0-9]*)
        echo "error: malformed supervise pidfile at $pid_file; use --restart after fixing the pidfile" >&2
        return 1
        ;;
      *)
        if ! host_run_pid_is_dead "$pid"; then
          echo "error: supervise pid $pid from $pid_file is still running for durable root $HOST_RUN_DURABLE_ROOT; use --restart to replace it" >&2
          return 1
        fi
        rm -f "$pid_file"
        ;;
    esac
  fi
  if ( set -C; printf '%s\n' "$$" > "$pid_file" ) 2>/dev/null; then
    wrote=1
  fi
  if [ "$wrote" -eq 1 ]; then
    return 0
  fi
  pid="$(sed -n '1p' "$pid_file" 2>/dev/null || true)"
  case "$pid" in
    ''|*[!0-9]*)
      echo "error: could not claim supervise pidfile at $pid_file" >&2
      ;;
    *)
      echo "error: supervise pid $pid claimed durable root $HOST_RUN_DURABLE_ROOT before launch; use --restart to replace it" >&2
      ;;
  esac
  return 1
}

host_run_print_package_roots() {
  local root
  for root in "${HOST_RUN_PACKAGE_ROOTS[@]}"; do
    printf '%s\n' "$root"
  done
}

host_run_supervise_contract() {
  host_run_parse_supervise_args "$@" || return $?
  host_run_hydrate_platform_source || return $?
  host_run_validate_shape || return $?
  host_run_build_package_roots || return $?
  if [ -n "${FKST_RATE_POOL_ROOT:-}" ]; then
    case "$FKST_RATE_POOL_ROOT" in
      /*) ;;
      *)
        echo "error: FKST_RATE_POOL_ROOT must be an absolute host-stable directory path" >&2
        return 1
        ;;
    esac
  fi

  host_run_restart_prior || return $?
  export FKST_RUNTIME_ROOT="$HOST_RUN_RUNTIME_ROOT"
  export FKST_DURABLE_ROOT="$HOST_RUN_DURABLE_ROOT"

  local args=() rootdir
  args=("$BIN" supervise --project-root "$HOST_RUN_PROJECT_ROOT")
  for rootdir in "${HOST_RUN_PACKAGE_ROOTS[@]}"; do
    args+=(--package-root "$rootdir")
  done
  args+=(--framework-bin "$BIN")

  echo "BIN=$BIN"
  echo "FKST_RUNTIME_ROOT=$FKST_RUNTIME_ROOT${HOST_RUN_RUNTIME_LABEL:+ ($HOST_RUN_RUNTIME_LABEL)}"
  echo "FKST_DURABLE_ROOT=$FKST_DURABLE_ROOT"
  if [ -n "${FKST_RATE_POOL_ROOT:-}" ]; then echo "FKST_RATE_POOL_ROOT=$FKST_RATE_POOL_ROOT"; fi
  if [ -n "${FKST_GITHUB_WRITE:-}" ]; then echo "FKST_GITHUB_WRITE=$FKST_GITHUB_WRITE"; else echo "FKST_GITHUB_WRITE=<unset> (dry-run)"; fi
  echo "project_root=$HOST_RUN_PROJECT_ROOT"
  echo "platform_root=$HOST_RUN_PLATFORM_ROOT"
  echo "package_roots:"
  host_run_print_package_roots | sed 's/^/  /'
  echo "This starts the real supervise event loop in the foreground. Press Ctrl-C to stop."
  echo "exec: ${args[*]}"
  host_run_claim_supervise_slot || return $?
  exec "${args[@]}"
}
