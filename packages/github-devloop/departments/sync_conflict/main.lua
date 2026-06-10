local core = require("core")

local M = {}

M.spec = {
  consumes = { "devloop_sync_conflict" },
  produces = {},
  stall_window = "10m",
}

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
    core.log_line("warn", "sync_conflict", "branch-sync", "CLEANUP", {
      "worktree=" .. tostring(worktree),
      "reason=" .. core._one_line(result.stderr or ""),
    })
  end
end

local function with_temp_worktree(conflict, fn)
  local runtime = runtime_root()
  local worktree = core.branch_sync_worktree_path(
    runtime,
    conflict.repo,
    conflict.upstream_branch,
    conflict.integration_branch,
    conflict.integration_sha
  )
  local add_result = exec_sync({ cmd = core.git_worktree_add_detached_cmd(worktree, conflict.integration_sha), timeout = 60 })
  if add_result.exit_code ~= 0 then
    error("github-devloop: git worktree add failed: " .. tostring(add_result.stderr))
  end

  local ok, result = pcall(fn, worktree, runtime)
  cleanup_worktree(worktree)
  if not ok then
    error(result)
  end
  return result
end

local function require_clean_resolution(worktree)
  local unmerged = run_git(core.git_unmerged_paths_cmd(worktree), 30, "git unmerged path check")
  if tostring(unmerged.stdout or "") ~= "" then
    error("github-devloop: sync conflict remains unresolved")
  end
  run_git(core.git_diff_check_cmd(worktree), 30, "git diff check")
  run_git(core.git_diff_cached_check_cmd(worktree), 30, "git cached diff check")
end

local function commit_resolution(worktree, runtime, conflict)
  run_git(core.git_add_all_cmd(worktree), 30, "git add")
  local unmerged = run_git(core.git_unmerged_paths_cmd(worktree), 30, "git unmerged path check before commit")
  if tostring(unmerged.stdout or "") ~= "" then
    error("github-devloop: sync conflict remains unresolved before commit")
  end
  run_git(core.git_diff_cached_check_cmd(worktree), 30, "git cached diff check before commit")
  local message_file = core.branch_sync_message_file(
    runtime,
    conflict.repo,
    conflict.upstream_branch,
    conflict.integration_branch,
    conflict.upstream_sha,
    conflict.integration_sha
  )
  file.write(message_file, core.sync_commit_message(
    conflict.repo,
    conflict.upstream_branch,
    conflict.integration_branch,
    conflict.upstream_sha,
    conflict.integration_sha,
    "resolved"
  ))
  run_git(core.git_commit_message_file_cmd(worktree, message_file), 60, "git sync commit")
end

local function push_if_real(conflict, worktree)
  if core.write_mode() ~= "real" then
    core.log_line("info", "sync_conflict", "branch-sync", "OUTBOUND", {
      "mode=dry-run",
      "repo=" .. tostring(conflict.repo),
      "upstream=" .. tostring(conflict.upstream_branch),
      "integration=" .. tostring(conflict.integration_branch),
      "upstream_sha=" .. tostring(conflict.upstream_sha),
      "integration_sha=" .. tostring(conflict.integration_sha),
      "reason=resolved branch sync push requires FKST_GITHUB_WRITE=1",
    })
    return
  end

  core.assert_trusted_bot_configured()
  fetch_branch(conflict.integration_branch)
  local rechecked_integration_sha = remote_head(conflict.integration_branch)
  if rechecked_integration_sha ~= conflict.integration_sha then
    core.log_cas_decision("sync_conflict", "branch-sync", {
      state = "integration",
      version = rechecked_integration_sha,
    }, "resolved", "push", "skip-foreign(head)", "integration head changed before resolved push")
    return
  end

  local merge_head = trim_stdout(run_git(core.git_head_sha_cmd(worktree), 30, "git resolved sync head"))
  if not core.is_safe_head_sha(merge_head) then
    error("github-devloop: unsafe resolved branch sync head")
  end
  run_git(core.git_push_worktree_branch_update_cmd(worktree, conflict.integration_branch), 120, "git resolved branch sync push")
  fetch_branch(conflict.integration_branch)
  local pushed_head = remote_head(conflict.integration_branch)
  if pushed_head ~= merge_head then
    error("github-devloop: resolved branch sync push verification failed")
  end
  core.log_apply("sync_conflict", "branch-sync", "synced", conflict.upstream_sha, {}, {})
end

function pipeline(event)
  local conflict = event.payload or {}
  if not core.is_supported_sync_conflict(conflict) then
    core.log_entry("sync_conflict", event, "branch-sync", conflict.dedup_key)
    core.log_cas_decision("sync_conflict", "branch-sync", { state = nil, version = nil }, "conflict", "resolved", "skip-foreign(payload)", "unsupported sync conflict payload")
    return
  end
  core.log_entry("sync_conflict", event, "branch-sync", conflict.dedup_key)

  with_lock(core.branch_sync_lock_key(conflict.repo, conflict.upstream_branch, conflict.integration_branch), function()
    fetch_branch(conflict.upstream_branch)
    fetch_branch(conflict.integration_branch)
    local upstream_sha = remote_head(conflict.upstream_branch)
    local integration_sha = remote_head(conflict.integration_branch)
    if integration_sha ~= conflict.integration_sha then
      core.log_cas_decision("sync_conflict", "branch-sync", { state = "integration", version = integration_sha }, "conflict", "resolved", "skip-stale(integration-head)", "integration head advanced after conflict event")
      return
    end
    if is_ancestor(upstream_sha, integration_sha) then
      core.log_cas_decision("sync_conflict", "branch-sync", { state = "synced", version = integration_sha }, "conflict", "resolved", "skip-idempotent(upstream-ancestor)", "conflict resolved elsewhere")
      return
    end

    local active_conflict = {
      schema = conflict.schema,
      repo = conflict.repo,
      upstream_branch = conflict.upstream_branch,
      integration_branch = conflict.integration_branch,
      upstream_sha = upstream_sha,
      integration_sha = conflict.integration_sha,
      dedup_key = conflict.dedup_key,
      source_ref = conflict.source_ref,
    }

    with_temp_worktree(active_conflict, function(worktree, runtime)
      local merge_result = exec_sync({ cmd = core.git_merge_no_ff_cmd(worktree, active_conflict.upstream_sha), timeout = 120 })
      if merge_result.exit_code == 0 then
        error("github-devloop: sync conflict event replayed without merge conflict")
      end
      local unmerged = run_git(core.git_unmerged_paths_cmd(worktree), 30, "git unmerged path check")
      if tostring(unmerged.stdout or "") == "" then
        error("github-devloop: sync conflict merge failed without unmerged paths")
      end

      core.log_codex_start("sync_conflict", "branch-sync", "sync-conflict")
      local result = spawn_codex_sync({
        prompt = core.build_sync_conflict_prompt(active_conflict),
        worktree = worktree,
      })
      if type(result) ~= "table" or result.exit_code ~= 0 then
        local stderr = type(result) == "table" and result.stderr or "nil result"
        core.log_codex_result("sync_conflict", "branch-sync", "sync-conflict", result, nil, stderr, {
          queue = event.queue,
          source_ref = conflict.source_ref,
          terminal = false,
        })
        error("github-devloop: sync conflict codex failed: " .. tostring(stderr))
      end
      core.log_codex_result("sync_conflict", "branch-sync", "sync-conflict", result, "result=completed", nil)

      require_clean_resolution(worktree)
      commit_resolution(worktree, runtime, active_conflict)
      push_if_real(active_conflict, worktree)
    end)
  end)
end

return M
