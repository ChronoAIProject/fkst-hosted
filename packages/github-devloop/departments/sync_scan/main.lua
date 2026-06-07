local core = require("core")

local M = {}

M.spec = {
  consumes = { "devloop_branch_tick" },
  produces = { "devloop_sync_conflict" },
  fanout = { "devloop_branch_tick" },
  stall_window = "10m",
}

local function require_repo(repo)
  local value = tostring(repo or "")
  if value == "" or core.safe_repo(value) ~= value then
    error("github-devloop: FKST_GITHUB_REPO is required for branch sync")
  end
  return value
end

local function run_git(cmd, timeout, error_class)
  local result = exec_sync({ cmd = cmd, timeout = timeout or 30 })
  if result.exit_code ~= 0 then
    error("github-devloop: " .. error_class .. " failed: " .. tostring(result.stderr))
  end
  return result
end

local function trim_stdout(result)
  return tostring(result.stdout or ""):gsub("%s+$", "")
end

local function fetch_branch(branch)
  run_git(core.git_fetch_branch_cmd("origin", branch), 60, "git branch fetch")
end

local function remote_head(branch)
  local result = run_git(core.git_remote_branch_head_cmd("origin", branch), 30, "git remote branch head")
  local head = trim_stdout(result)
  if not core.is_safe_head_sha(head) then
    error("github-devloop: unsafe remote branch head")
  end
  return head
end

local function is_ancestor(ancestor_sha, descendant_sha)
  local result = exec_sync({ cmd = core.git_is_ancestor_cmd(ancestor_sha, descendant_sha), timeout = 30 })
  if result.exit_code == 0 then
    return true
  end
  if result.exit_code == 1 then
    return false
  end
  error("github-devloop: git ancestor check failed: " .. tostring(result.stderr))
end

local function runtime_root()
  local result = run_git(core.read_runtime_root_cmd(), 30, "FKST_RUNTIME_ROOT read")
  return result.stdout
end

local function cleanup_worktree(worktree)
  if worktree == nil then
    return
  end
  local result = exec_sync({ cmd = core.git_worktree_remove_cmd(worktree), timeout = 60 })
  if result.exit_code ~= 0 then
    core.log_line("warn", "sync_scan", "branch-sync", "CLEANUP", {
      "worktree=" .. tostring(worktree),
      "reason=" .. core._one_line(result.stderr or ""),
    })
  end
end

local function with_temp_worktree(runtime, repo, upstream, integration, integration_sha, fn)
  local worktree = core.branch_sync_worktree_path(runtime, repo, upstream, integration, integration_sha)
  local add_result = exec_sync({ cmd = core.git_worktree_add_detached_cmd(worktree, integration_sha), timeout = 60 })
  if add_result.exit_code ~= 0 then
    error("github-devloop: git worktree add failed: " .. tostring(add_result.stderr))
  end

  local ok, result = pcall(fn, worktree)
  cleanup_worktree(worktree)
  if not ok then
    error(result)
  end
  return result
end

local function write_sync_commit(worktree, runtime, repo, upstream, integration, upstream_sha, integration_sha, result)
  local message_file = core.branch_sync_message_file(runtime, repo, upstream, integration, upstream_sha, integration_sha)
  file.write(message_file, core.sync_commit_message(repo, upstream, integration, upstream_sha, integration_sha, result))
  run_git(core.git_commit_message_file_cmd(worktree, message_file), 60, "git sync commit")
end

local function raise_conflict(repo, upstream, integration, upstream_sha, integration_sha)
  local payload = {
    schema = "github-devloop.v1",
    repo = repo,
    upstream_branch = upstream,
    integration_branch = integration,
    upstream_sha = upstream_sha,
    integration_sha = integration_sha,
    dedup_key = core.branch_sync_dedup_key(repo, upstream, integration, upstream_sha),
    source_ref = core.branch_sync_source_ref(repo, upstream, integration),
  }
  core.log_raise("sync_scan", "branch-sync", "devloop_sync_conflict", payload)
end

local function push_if_real(repo, upstream, integration, upstream_sha, integration_sha, worktree)
  if core.write_mode() ~= "real" then
    core.log_line("info", "sync_scan", "branch-sync", "OUTBOUND", {
      "mode=dry-run",
      "repo=" .. tostring(repo),
      "upstream=" .. tostring(upstream),
      "integration=" .. tostring(integration),
      "upstream_sha=" .. tostring(upstream_sha),
      "integration_sha=" .. tostring(integration_sha),
      "reason=branch sync push requires FKST_GITHUB_WRITE=1",
    })
    return
  end

  core.assert_trusted_bot_configured()
  fetch_branch(integration)
  local rechecked_integration_sha = remote_head(integration)
  if rechecked_integration_sha ~= integration_sha then
    core.log_cas_decision("sync_scan", "branch-sync", {
      state = "integration",
      version = rechecked_integration_sha,
    }, "sync", "push", "skip-foreign(head)", "integration head changed before push")
    return
  end

  local merge_head = trim_stdout(run_git(core.git_head_sha_cmd(worktree), 30, "git sync head"))
  if not core.is_safe_head_sha(merge_head) then
    error("github-devloop: unsafe branch sync merge head")
  end
  run_git(core.git_push_worktree_branch_update_cmd(worktree, integration), 120, "git branch sync push")
  fetch_branch(integration)
  local pushed_head = remote_head(integration)
  if pushed_head ~= merge_head then
    error("github-devloop: branch sync push verification failed")
  end
  core.log_apply("sync_scan", "branch-sync", "synced", upstream_sha, {}, {})
end

local function fast_forward_sync(repo, upstream, integration, upstream_sha, integration_sha)
  local runtime = runtime_root()
  with_temp_worktree(runtime, repo, upstream, integration, integration_sha, function(worktree)
    run_git(core.git_fast_forward_cmd(worktree, upstream_sha), 120, "git branch sync fast-forward")
    push_if_real(repo, upstream, integration, upstream_sha, integration_sha, worktree)
  end)
end

function pipeline(event)
  core.log_entry("sync_scan", event, "branch-sync", event and event.queue or "")
  local branches = core.branch_config()
  local cfg = core.devloop_config()
  local repo = require_repo(cfg.repo)

  if branches.integration == branches.upstream then
    core.log_cas_decision("sync_scan", "branch-sync", { state = "same-branch", version = branches.upstream }, "tick", "sync", "skip-idempotent(same-branch)", "integration branch equals upstream branch")
    return
  end

  with_lock(core.branch_sync_lock_key(repo, branches.upstream, branches.integration), function()
    fetch_branch(branches.upstream)
    fetch_branch(branches.integration)
    local upstream_sha = remote_head(branches.upstream)
    local integration_sha = remote_head(branches.integration)

    if is_ancestor(upstream_sha, integration_sha) then
      core.log_cas_decision("sync_scan", "branch-sync", { state = "synced", version = integration_sha }, "tick", "sync", "skip-idempotent(upstream-ancestor)", "upstream head is already contained in integration")
      return
    end
    if is_ancestor(integration_sha, upstream_sha) then
      fast_forward_sync(repo, branches.upstream, branches.integration, upstream_sha, integration_sha)
      return
    end

    local runtime = runtime_root()
    with_temp_worktree(runtime, repo, branches.upstream, branches.integration, integration_sha, function(worktree)
      local merge_result = exec_sync({ cmd = core.git_merge_no_ff_cmd(worktree, upstream_sha), timeout = 120 })
      if merge_result.exit_code == 0 then
        write_sync_commit(worktree, runtime, repo, branches.upstream, branches.integration, upstream_sha, integration_sha, "clean")
        push_if_real(repo, branches.upstream, branches.integration, upstream_sha, integration_sha, worktree)
        return
      end

      local unmerged = exec_sync({ cmd = core.git_unmerged_paths_cmd(worktree), timeout = 30 })
      if unmerged.exit_code ~= 0 then
        error("github-devloop: git unmerged path check failed: " .. tostring(unmerged.stderr))
      end
      if tostring(unmerged.stdout or "") ~= "" then
        raise_conflict(repo, branches.upstream, branches.integration, upstream_sha, integration_sha)
        return
      end
      error("github-devloop: git sync merge failed without conflicts: " .. tostring(merge_result.stderr))
    end)
  end)
end

return M
