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

host_run_resolve_target_platform_roots() {
  local output line
  output="$(python3 - "$HOST_RUN_PROJECT_ROOT" "$HOST_RUN_PLATFORM_PACKAGES" "$HOST_RUN_PLATFORM_ROOT" <<'PY'
import re
import shutil
import subprocess
import sys
import tomllib
from glob import glob
from pathlib import Path
from urllib.parse import unquote, urlparse

ID_RE = re.compile(r"[A-Za-z0-9._-]+")
REV_RE = re.compile(r"[0-9a-fA-F]{40}")


def fail(message: str) -> None:
    print(f"error: {message}", file=sys.stderr)
    raise SystemExit(1)


def run_git(args: list[str], *, cwd: Path | None = None) -> None:
    try:
        subprocess.run(["git", *args], cwd=cwd, check=True)
    except FileNotFoundError:
        fail("git is required to hydrate host external sources")
    except subprocess.CalledProcessError as exc:
        fail(f"git {' '.join(args)} failed with exit {exc.returncode}")


def git_output(args: list[str], *, cwd: Path) -> str:
    try:
        result = subprocess.run(
            ["git", *args],
            cwd=cwd,
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            check=True,
        )
    except FileNotFoundError:
        fail("git is required to hydrate host external sources")
    except subprocess.CalledProcessError as exc:
        fail(f"git {' '.join(args)} failed with exit {exc.returncode}")
    return result.stdout.strip()


def git_output_optional(args: list[str], *, cwd: Path) -> str | None:
    try:
        result = subprocess.run(
            ["git", *args],
            cwd=cwd,
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            check=False,
        )
    except FileNotFoundError:
        fail("git is required to hydrate host external sources")
    if result.returncode != 0:
        return None
    return result.stdout.strip()


def remove_target(target: Path, run_root: Path) -> None:
    if target.parent != run_root:
        fail(f"refusing to remove external source outside .fkst/run: {target}")
    if target.is_symlink() or target.is_file():
        target.unlink()
    elif target.exists():
        shutil.rmtree(target)


def list_of_tables(data: dict[str, object], key: str) -> list[dict[str, object]]:
    entries = data.get(key, [])
    if isinstance(entries, dict):
        entries = [entries]
    if not isinstance(entries, list):
        fail(f"fkst.workspace.toml {key} must be a table array")
    out: list[dict[str, object]] = []
    for entry in entries:
        if not isinstance(entry, dict):
            fail(f"fkst.workspace.toml {key} entries must be tables")
        out.append(entry)
    return out


def string_list(value: object, field: str) -> list[str]:
    if not isinstance(value, list):
        fail(f"fkst.workspace.toml {field} must be a string array")
    out: list[str] = []
    for item in value:
        if not isinstance(item, str) or not item:
            fail(f"fkst.workspace.toml {field} must contain only non-empty strings")
        out.append(item)
    return out


def lock_sources(lock_path: Path) -> list[dict[str, object]]:
    try:
        data = tomllib.loads(lock_path.read_text(encoding="utf-8"))
    except tomllib.TOMLDecodeError as exc:
        fail(f"invalid fkst lockfile: {lock_path}: {exc}")
    sources = data.get("external_source", [])
    if isinstance(sources, dict):
        sources = [sources]
    if not isinstance(sources, list):
        fail("fkst.lock external_source must be a table array")
    for source in sources:
        if not isinstance(source, dict):
            fail("fkst.lock external_source entries must be tables")
    return sources


def validate_source(source: dict[str, object]) -> tuple[str, str, str]:
    source_id = source.get("id")
    git_url = source.get("git")
    resolved = source.get("resolved")
    rev = resolved.get("rev") if isinstance(resolved, dict) else None
    if not isinstance(source_id, str) or not ID_RE.fullmatch(source_id) or source_id in {".", ".."}:
        fail("fkst.lock external_source has invalid id")
    if not isinstance(git_url, str) or not git_url:
        fail(f"fkst.lock external_source(id={source_id}) is missing git")
    if not isinstance(rev, str) or not REV_RE.fullmatch(rev):
        fail(f"fkst.lock external_source(id={source_id}) is missing resolved.rev as a full git SHA")
    return source_id, git_url, rev.lower()


def is_scp_like_url(value: str) -> bool:
    return bool(re.match(r"^[A-Za-z0-9_.-]+@[^:]+:", value))


def git_ref_names(value: str, *, base: Path) -> set[str]:
    text = value.rstrip("/")
    refs = {text}
    parsed = urlparse(text)
    if parsed.scheme == "file":
        refs.add(str(Path(unquote(parsed.path)).resolve()))
    elif "://" not in text and not is_scp_like_url(text):
        path = Path(text)
        if not path.is_absolute():
            path = base / path
        refs.add(str(path.resolve()))
    return refs


def same_path(left: Path, right: Path) -> bool:
    try:
        return left.resolve() == right.resolve()
    except OSError:
        return False


def trusted_platform_identity(platform_root: Path) -> tuple[str, set[str]]:
    if not platform_root.is_dir():
        fail(f"trusted --platform-root does not exist: {platform_root}")
    head = git_output(["rev-parse", "HEAD"], cwd=platform_root).lower()
    if not REV_RE.fullmatch(head):
        fail(f"trusted --platform-root HEAD is not a full git SHA: {platform_root}")
    top = Path(git_output(["rev-parse", "--show-toplevel"], cwd=platform_root)).resolve()
    refs = git_ref_names(str(top), base=top)
    refs.update(git_ref_names(str(platform_root), base=top))
    origin = git_output_optional(["config", "--get", "remote.origin.url"], cwd=platform_root)
    if origin:
        refs.update(git_ref_names(origin, base=top))
    return head, refs


def read_workspace(workspace_path: Path) -> dict[str, object]:
    if not workspace_path.is_file():
        fail(f"target fkst.workspace.toml is required for host supervise: {workspace_path}")
    try:
        data = tomllib.loads(workspace_path.read_text(encoding="utf-8"))
    except tomllib.TOMLDecodeError as exc:
        fail(f"invalid target fkst.workspace.toml: {workspace_path}: {exc}")
    if not isinstance(data, dict):
        fail("target fkst.workspace.toml must be a TOML table")
    return data


def hydrate_source(project_root: Path, source_id: str, git_url: str, rev: str) -> Path:
    run_root = project_root / ".fkst" / "run"
    target = run_root / source_id
    if target.exists() or target.is_symlink():
        if not target.is_symlink() and (target / ".git").is_dir():
            current = git_output(["rev-parse", "HEAD"], cwd=target).lower()
            origin = git_output_optional(["config", "--get", "remote.origin.url"], cwd=target)
            if current == rev and origin == git_url:
                return target
        remove_target(target, run_root)

    target.parent.mkdir(parents=True, exist_ok=True)
    run_git(["clone", "--quiet", "--no-checkout", git_url, str(target)])
    run_git(["checkout", "--quiet", rev], cwd=target)
    current = git_output(["rev-parse", "HEAD"], cwd=target).lower()
    if current != rev:
        fail(f"external source {source_id} checkout is at {current}, expected {rev}")
    return target


project_root = Path(sys.argv[1]).resolve()
requested_packages = [item for item in sys.argv[2].split() if item]
trusted_platform_root = Path(sys.argv[3]).resolve()
workspace_path = project_root / "fkst.workspace.toml"
workspace = read_workspace(workspace_path)
workspace_packages: dict[str, str] = {}
workspace_table = workspace.get("workspace", {})
if isinstance(workspace_table, dict):
    unit_globs = workspace_table.get("units", [])
    if unit_globs is None:
        unit_globs = []
    for pattern in string_list(unit_globs, "workspace.units"):
        if pattern.startswith("/") or ".." in Path(pattern).parts:
            fail("fkst.workspace.toml workspace.units entries must be safe relative globs")
        for match in glob(str(project_root / pattern)):
            package_root = Path(match)
            if package_root.is_dir() and (package_root / "fkst.toml").is_file():
                workspace_packages[package_root.name] = "workspace"
for package in list_of_tables(workspace, "package"):
    name = package.get("name")
    source = package.get("source", "workspace")
    if isinstance(name, str) and source == "workspace":
        workspace_packages[name] = "workspace"

external_sources: dict[str, dict[str, object]] = {}
for source in list_of_tables(workspace, "external_sources"):
    source_id = source.get("id")
    git_url = source.get("git")
    if not isinstance(source_id, str) or not ID_RE.fullmatch(source_id) or source_id in {".", ".."}:
        fail("fkst.workspace.toml external_sources has invalid id")
    if not isinstance(git_url, str) or not git_url:
        fail(f"fkst.workspace.toml external_sources(id={source_id}) is missing git")
    packages = string_list(source.get("packages", []), f"external_sources(id={source_id}).packages")
    external_sources[source_id] = {"git": git_url, "packages": packages}

lock_by_id: dict[str, tuple[str, str]] = {}
selected: list[tuple[str, str, str | None]] = []
needed_external_source_ids: set[str] = set()

for package in requested_packages:
    matches: list[tuple[str, str | None]] = []
    if package in workspace_packages:
        matches.append(("workspace", None))
    for source_id, source in external_sources.items():
        if package in source["packages"]:
            matches.append(("external", source_id))
    if not matches:
        fail(f"target fkst.workspace.toml does not declare platform package '{package}'")
    if len(matches) > 1:
        fail(f"ambiguous target fkst.workspace.toml platform package '{package}'")
    kind, source_id = matches[0]
    selected.append((package, kind, source_id))
    if source_id is not None:
        needed_external_source_ids.add(source_id)

if needed_external_source_ids:
    lock_path = project_root / "fkst.lock"
    if not lock_path.is_file():
        fail(f"target fkst.lock is required for external platform packages: {lock_path}")
    for source in lock_sources(lock_path):
        source_id, git_url, rev = validate_source(source)
        lock_by_id[source_id] = (git_url, rev)

source_roots: dict[str, Path] = {}
trusted_platform_head = ""
trusted_platform_refs: set[str] = set()
if needed_external_source_ids:
    trusted_platform_head, trusted_platform_refs = trusted_platform_identity(trusted_platform_root)

for source_id in sorted(needed_external_source_ids):
    if source_id not in lock_by_id:
        fail(f"fkst.lock has no external_source(id={source_id}) for target platform packages")
    expected_git = external_sources[source_id]["git"]
    git_url, rev = lock_by_id[source_id]
    if git_url != expected_git:
        fail(f"fkst.workspace.toml external_sources(id={source_id}) git does not match fkst.lock")
    if rev != trusted_platform_head:
        fail(f"fkst.lock external_source(id={source_id}) resolved.rev does not match trusted --platform-root HEAD")
    if trusted_platform_refs.isdisjoint(git_ref_names(git_url, base=project_root)):
        fail(f"fkst.workspace.toml external_sources(id={source_id}) git does not match trusted --platform-root")
    source_roots[source_id] = hydrate_source(project_root, source_id, git_url, rev)

package_roots: list[Path] = []
platform_roots: set[Path] = set()
for package, kind, source_id in selected:
    if kind == "workspace":
        if not same_path(project_root, trusted_platform_root):
            fail(f"workspace platform package '{package}' requires trusted --platform-root")
        root = project_root / "packages" / package
        platform_roots.add(project_root)
    else:
        assert source_id is not None
        root = source_roots[source_id] / "packages" / package
        platform_roots.add(source_roots[source_id])
    if not root.is_dir():
        fail(f"missing target platform package '{package}' at {root}")
    package_roots.append(root)

if len(platform_roots) != 1:
    fail("target platform packages must resolve to exactly one workspace or external source")

print(f"PLATFORM_ROOT={next(iter(platform_roots))}")
for root in package_roots:
    print(f"PACKAGE_ROOT={root}")
PY
)" || return $?

  while IFS= read -r line || [ -n "$line" ]; do
    case "$line" in
      PLATFORM_ROOT=*) HOST_RUN_PLATFORM_ROOT="${line#PLATFORM_ROOT=}" ;;
      PACKAGE_ROOT=*) HOST_RUN_PACKAGE_ROOTS+=("${line#PACKAGE_ROOT=}") ;;
      *) echo "error: unexpected platform resolver output: $line" >&2; return 1 ;;
    esac
  done <<< "$output"
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
  host_run_resolve_target_platform_roots || return 1
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
