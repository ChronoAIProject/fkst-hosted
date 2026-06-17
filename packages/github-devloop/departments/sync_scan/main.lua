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

local function run_required(result, error_class)
  if result.exit_code ~= 0 then
    error("github-devloop: " .. error_class .. " failed: " .. tostring(result.stderr))
  end
  return result
end

local function require_git_ok(result, error_class)
  if result.exit_code ~= 0 then
    error("github-devloop: " .. error_class .. " failed: " .. tostring(result.stderr))
  end
  return result
end

local function trim_stdout(result)
  return tostring(result.stdout or ""):gsub("%s+$", "")
end

local function fetch_branch(branch)
  run_required(core.git_fetch_branch("origin", branch, 60), "branch fetch")
end

local function fetch_branches(repo, branches)
  core.with_repo_ref_store_lock(repo, function()
    for _, branch in ipairs(branches) do
      fetch_branch(branch)
    end
  end)
end

local function remote_head(branch)
  local result = run_required(core.git_remote_branch_head("origin", branch, 30), "remote branch head")
  local head = trim_stdout(result)
  if not core.is_safe_head_sha(head) then
    error("github-devloop: unsafe remote branch head")
  end
  return head
end

local function is_ancestor(ancestor_sha, descendant_sha)
  local result = core.git_is_ancestor(ancestor_sha, descendant_sha, 30)
  if result.exit_code == 0 then
    return true
  end
  if result.exit_code == 1 then
    return false
  end
  error("github-devloop: ancestor check failed: " .. tostring(result.stderr))
end

local function trees_equal(sha_a, sha_b)
  local result = core.git_trees_equal_quiet(sha_a, sha_b, 30)
  if result.exit_code == 0 then
    return true
  end
  if result.exit_code == 1 then
    return false
  end
  error("github-devloop: tree compare failed: " .. tostring(result.stderr))
end

local function runtime_root()
  local result = run_required(exec_sync({ cmd = core.read_runtime_root_cmd(), timeout = 30 }), "FKST_RUNTIME_ROOT read")
  return result.stdout
end

local function cleanup_worktree(worktree)
  if worktree == nil then
    return
  end
  local result = core.git_worktree_remove(worktree, 60)
  if result.exit_code ~= 0 then
    core.log_line("warn", "sync_scan", "branch-sync", "CLEANUP", {
      "worktree=" .. tostring(worktree),
      "reason=" .. core._one_line(result.stderr or ""),
    })
  end
end

local function with_temp_worktree(runtime, repo, upstream, integration, integration_sha, fn)
  local worktree = core.branch_sync_worktree_path(runtime, repo, upstream, integration, integration_sha)
  local plan = core.git_worktree_add_detached_plan(worktree, integration_sha)
  run_required(exec_sync({ cmd = core.mkdir_p_cmd(plan.parent_dir), timeout = 30 }), "worktree parent directory setup")
  require_git_ok(core.git_worktree_add_detached(plan.worktree, plan.sha, 60), "worktree add")

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
  require_git_ok(core.git_commit_message_file(worktree, message_file, 60), "sync commit")
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
  fetch_branches(repo, { integration })
  local rechecked_integration_sha = remote_head(integration)
  if rechecked_integration_sha ~= integration_sha then
    core.log_cas_decision("sync_scan", "branch-sync", {
      state = "integration",
      version = rechecked_integration_sha,
    }, "sync", "push", "skip-foreign(head)", "integration head changed before push")
    return
  end

  local merge_head = trim_stdout(run_required(core.git_head_sha(worktree, 30), "sync head"))
  if not core.is_safe_head_sha(merge_head) then
    error("github-devloop: unsafe branch sync merge head")
  end
  require_git_ok(core.git_push_worktree_branch_update(worktree, integration, 120), "branch sync push")
  fetch_branches(repo, { integration })
  local pushed_head = remote_head(integration)
  if pushed_head ~= merge_head then
    error("github-devloop: branch sync push verification failed")
  end
  core.log_apply("sync_scan", "branch-sync", "synced", upstream_sha, {}, {})
end

local function converge_integration_to_upstream(repo, upstream, integration, upstream_sha, integration_sha)
  if core.write_mode() ~= "real" then
    core.log_line("info", "sync_scan", "branch-sync", "OUTBOUND", {
      "mode=dry-run",
      "repo=" .. tostring(repo),
      "upstream=" .. tostring(upstream),
      "integration=" .. tostring(integration),
      "upstream_sha=" .. tostring(upstream_sha),
      "integration_sha=" .. tostring(integration_sha),
      "reason=branch sync converge reset requires FKST_GITHUB_WRITE=1",
    })
    return
  end

  core.assert_trusted_bot_configured()
  fetch_branches(repo, { integration })
  local rechecked_integration_sha = remote_head(integration)
  if rechecked_integration_sha ~= integration_sha then
    core.log_cas_decision("sync_scan", "branch-sync", {
      state = "integration",
      version = rechecked_integration_sha,
    }, "sync", "converge", "skip-foreign(head)", "integration head changed before converge reset")
    return
  end

  if not trees_equal(upstream_sha, integration_sha) then
    core.log_cas_decision("sync_scan", "branch-sync", {
      state = "diverged",
      version = integration_sha,
    }, "sync", "converge", "skip-idempotent(tree-changed)", "branch trees changed before converge reset")
    return
  end

  require_git_ok(core.git_push_branch_force_with_lease(integration, upstream_sha, integration_sha, 120), "branch sync converge")
  fetch_branches(repo, { integration })
  local pushed_head = remote_head(integration)
  if pushed_head ~= upstream_sha then
    error("github-devloop: branch sync converge verification failed")
  end
  core.log_apply("sync_scan", "branch-sync", "converged", upstream_sha, {}, {})
end

local function fast_forward_sync(repo, upstream, integration, upstream_sha, integration_sha)
  local runtime = runtime_root()
  with_temp_worktree(runtime, repo, upstream, integration, integration_sha, function(worktree)
    require_git_ok(core.git_fast_forward(worktree, upstream_sha, 120), "branch sync fast-forward")
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
    fetch_branches(repo, { branches.upstream, branches.integration })
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
    if trees_equal(upstream_sha, integration_sha) then
      converge_integration_to_upstream(repo, branches.upstream, branches.integration, upstream_sha, integration_sha)
      return
    end

    local runtime = runtime_root()
    with_temp_worktree(runtime, repo, branches.upstream, branches.integration, integration_sha, function(worktree)
      local merge_result = core.git_merge_no_ff(worktree, upstream_sha, 120)
      if merge_result.exit_code == 0 then
        write_sync_commit(worktree, runtime, repo, branches.upstream, branches.integration, upstream_sha, integration_sha, "clean")
        push_if_real(repo, branches.upstream, branches.integration, upstream_sha, integration_sha, worktree)
        return
      end

      local unmerged = core.git_unmerged_paths(worktree, 30)
      if unmerged.exit_code ~= 0 then
        error("github-devloop: unmerged path check failed: " .. tostring(unmerged.stderr))
      end
      if tostring(unmerged.stdout or "") ~= "" then
        raise_conflict(repo, branches.upstream, branches.integration, upstream_sha, integration_sha)
        return
      end
      error("github-devloop: sync merge failed without conflicts: " .. tostring(merge_result.stderr))
    end)
  end)
end

pipeline = core.wrap_pipeline_failure("sync_scan", pipeline)

return M
