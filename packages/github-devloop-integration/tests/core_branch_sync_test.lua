local git_mechanics = require("devloop.git_mechanics")
local h = require("tests.devloop_core_helpers")
local core = h.core
local t = h.t

local function shell_quote(value)
  return "'" .. tostring(value):gsub("'", "'\\''") .. "'"
end

local package_root = "packages/github-devloop-integration"

local function mkdir_p(path)
  local ok = os.execute("mkdir -p " .. shell_quote(path))
  if not (ok == true or ok == 0) then
    error("github-devloop: mkdir failed for " .. tostring(path))
  end
end

local function read_file(path)
  local handle = assert(io.open(path, "r"))
  local body = handle:read("*a")
  handle:close()
  return body
end

local function write_file(path, body)
  local handle = assert(io.open(path, "w"))
  handle:write(body)
  handle:close()
end

local function run_shell(cmd)
  local handle = assert(io.popen(cmd .. " 2>&1"))
  local output = handle:read("*a")
  local ok, _, status = handle:close()
  return {
    exit_code = ok and 0 or (status or 1),
    output = output or "",
  }
end

local function write_fetch_probe_git(path)
  write_file(path, [[#!/usr/bin/env bash
set -euo pipefail

state_dir="${FKST_TEST_FETCH_STATE:?}"
mkdir -p "$state_dir"

if [ "${1:-}" = "fetch" ]; then
  lock_path="${FKST_RUNTIME_ROOT:?}/locks/github-devloop/git/owner/repo/fetch/=lock"
  mkdir -p "$(dirname "$lock_path")"
  if [ ! -e "$lock_path" ]; then
    printf 'fetch entered without repo ref-store lock file\n' >> "$state_dir/violations"
    exit 43
  fi

  if ! mkdir "$state_dir/active" 2>/dev/null; then
    printf 'overlap on %s\n' "$*" >> "$state_dir/violations"
  else
    printf '%s\n' "$*" > "$state_dir/active/command"
  fi

  sleep 0.2

  rm -rf "$state_dir/active"
  exit 0
fi

if [ "${1:-}" = "rev-parse" ]; then
  case "$*" in
    *"refs/remotes/origin/"*) printf 'bbbb2222\n'; exit 0 ;;
  esac
fi

if [ "${1:-}" = "merge-base" ] && [ "${2:-}" = "--is-ancestor" ]; then
  exit 0
fi

if [ "${1:-}" = "rev-list" ] && [ "${2:-}" = "--count" ]; then
  printf '0\n'
  exit 0
fi

if [ "${1:-}" = "diff" ] && [ "${2:-}" = "--quiet" ]; then
  exit 0
fi

printf 'unexpected git command: %s\n' "$*" >&2
exit 44
]])
  local ok = os.execute("chmod +x " .. shell_quote(path))
  if not (ok == true or ok == 0) then
    error("github-devloop: chmod failed for git probe")
  end
end

return {
  test_branch_sync_identity_helpers = function()
    local source_ref = core.branch_sync_source_ref("owner/repo", "dev", "integration/dev")
    t.eq(source_ref.kind, "external")
    t.eq(source_ref.ref, "owner/repo#branch-sync/dev/integration/dev")

    t.eq(
      core.branch_sync_lock_key("owner/repo", "dev", "integration/dev"),
      "github-devloop/branch-sync/owner/repo/dev/integration/dev"
    )
    t.eq(
      git_mechanics.repo_ref_store_lock_key("owner/repo"),
      "github-devloop/git/owner/repo/fetch"
    )
    t.eq(
      core.branch_sync_dedup_key("owner/repo", "dev", "integration/dev", "abcdef1234"),
      "branch-sync/owner/repo/dev/integration/dev/abcdef1234"
    )
    t.eq(
      core.sync_commit_marker("owner/repo", "dev", "integration/dev", "abcdef1234", "fedcba4321", "clean"),
      '<!-- fkst:github-devloop:sync:v1 repo="owner/repo" upstream="dev" integration="integration/dev" upstream_sha="abcdef1234" integration_parent="fedcba4321" result="clean" -->'
    )
    local message = core.sync_commit_message("owner/repo", "dev", "integration/dev", "abcdef1234", "fedcba4321", "resolved")
    t.is_true(message:find("Sync dev into integration/dev", 1, true) == 1)
    t.is_true(message:find('result="resolved"', 1, true) ~= nil)

    t.eq(
      core.is_supported_sync_conflict({
        schema = "github-devloop.v1",
        repo = "owner/repo",
        upstream_branch = "dev",
        integration_branch = "integration/dev",
        upstream_sha = "abcdef1234",
        integration_sha = "fedcba4321",
        dedup_key = "branch-sync/owner/repo/dev/integration/dev/abcdef1234",
        source_ref = source_ref,
      }),
      true
    )
  end,

  test_branch_sync_rejects_unsafe_shapes = function()
    t.raises(function()
      core.branch_sync_lock_key("../repo", "dev", "integration/dev")
    end)
    t.raises(function()
      git_mechanics.repo_ref_store_lock_key("../repo")
    end)
    t.raises(function()
      core.branch_sync_source_ref("owner/repo", "../dev", "integration/dev")
    end)
    t.raises(function()
      core.branch_sync_dedup_key("owner/repo", "dev", "integration/dev", "not-a-sha")
    end)
    t.raises(function()
      core.sync_commit_marker("owner/repo", "dev", "integration/dev", "abcdef", "fedcba", "manual")
    end)
  end,

  test_branch_scan_departments_serialize_same_repo_fetch_sections = function()
    local bin = os.getenv("BIN")
    t.is_true(type(bin) == "string" and bin ~= "")

    local root = "/tmp/fkst-packages-test/github-devloop/ref-store-lock-" .. tostring(now())
    local runtime = root .. "/runtime"
    local state_dir = root .. "/state"
    local bin_dir = root .. "/bin"
    mkdir_p(runtime)
    mkdir_p(state_dir)
    mkdir_p(bin_dir)
    write_fetch_probe_git(bin_dir .. "/git")

    local pkg = package_root
    local event = shell_quote('{"queue":"devloop_branch_tick","payload":{"schema":"github-devloop.branch-tick.v1"}}')
    local env = table.concat({
      "env -u FKST_SUPERVISOR_PID",
      "FKST_RUNTIME_ROOT=" .. shell_quote(runtime),
      "FKST_TEST_FETCH_STATE=" .. shell_quote(state_dir),
      "FKST_GITHUB_REPO=owner/repo",
      "FKST_DEVLOOP_UPSTREAM_BRANCH=dev",
      "FKST_DEVLOOP_INTEGRATION_BRANCH=integration/dev",
      "FKST_DEVLOOP_ROLLUP_MERGE=auto",
      "FKST_GITHUB_WRITE=",
      "PATH=" .. shell_quote(bin_dir .. ":" .. tostring(os.getenv("PATH") or "")),
    }, " ")
    local function run_department_command(path, out)
      return env
        .. " "
        .. shell_quote(bin)
        .. " run "
        .. shell_quote(pkg .. "/" .. path)
        .. " --project-root "
        .. shell_quote(pkg)
        .. " --package-root "
        .. shell_quote(pkg)
        .. " --event "
        .. event
        .. " > "
        .. shell_quote(state_dir .. "/" .. out)
        .. " 2>&1"
    end

    local command = table.concat({
      run_department_command("departments/sync_scan/main.lua", "sync.out") .. " & sync_pid=$!",
      run_department_command("departments/rollup_scan/main.lua", "rollup.out") .. " & rollup_pid=$!",
      "wait \"$sync_pid\"; sync_rc=$?",
      "wait \"$rollup_pid\"; rollup_rc=$?",
      "printf '%s %s\\n' \"$sync_rc\" \"$rollup_rc\" > " .. shell_quote(state_dir .. "/status"),
      "exit $((sync_rc + rollup_rc))",
    }, "; ")

    local result = run_shell(command)

    t.eq(result.exit_code, 0, result.output)
    t.eq(read_file(state_dir .. "/status"), "0 0\n")
    local violations = io.open(state_dir .. "/violations", "r")
    if violations ~= nil then
      local body = violations:read("*a")
      violations:close()
      t.eq(body, "")
    end
  end,
}
