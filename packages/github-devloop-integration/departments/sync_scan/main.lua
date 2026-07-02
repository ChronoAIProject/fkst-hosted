local git_mechanics = require("devloop.git_mechanics")
local devloop_base = require("devloop.base")
local base_ids = require("devloop.base_ids")
local error_facts = require("contract.error_facts")
local core = require("core")
local git_adapter = require("forge.git")
local saga = require("workflow.saga")
local config = require("devloop.config")

local spec = {
  consumes = { "devloop_branch_tick" },
  produces = { "devloop_sync_conflict" },
  fanout = { "devloop_branch_tick" },
  stall_window = "10m",
}

local git = git_adapter.production_handle

local function done(_event)
  return false
end

local function require_repo(repo)
  local value = tostring(repo or "")
  if value == "" or base_ids.safe_repo(value) ~= value then
    error("github-devloop: FKST_GITHUB_REPO is required for branch sync")
  end
  return value
end

local function trim_stdout(result)
  return tostring(result.stdout or ""):gsub("%s+$", "")
end

local function trees_equal(sha_a, sha_b)
  local result = git_mechanics.git_trees_equal_quiet(core.git, sha_a, sha_b, 30)
  if result.exit_code == 0 then
    return true
  end
  if result.exit_code == 1 then
    return false
  end
  error("github-devloop: tree compare failed: " .. tostring(result.stderr))
end

local function cleanup_worktree(worktree)
  if worktree == nil then
    return
  end
  local result = core.git.worktree_remove(worktree, 60)
  if result.exit_code ~= 0 then
    core.log_line("warn", "sync_scan", "branch-sync", "CLEANUP", {
      "worktree=" .. tostring(worktree),
      "reason=" .. error_facts.one_line(result.stderr or ""),
    })
  end
end

local function with_temp_worktree(runtime, repo, upstream, integration, integration_sha, fn)
  local worktree = core.branch_sync_worktree_path(runtime, repo, upstream, integration, integration_sha)
  local plan = git("github-devloop").git_worktree_add_detached_plan(worktree, integration_sha)
  git_mechanics.run_required(exec_sync({ cmd = core.mkdir_p_cmd(plan.parent_dir), timeout = 30 }), "worktree parent directory setup")
  git_mechanics.run_required(git("github-devloop").git_worktree_add_detached(plan.worktree, plan.sha, 60), "worktree add")

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
  git_mechanics.run_required(core.git.commit_message_file(worktree, message_file, 60), "sync commit")
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
  if config.write_mode(core) ~= "real" then
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

  devloop_base.assert_trusted_bot_configured()
  git_mechanics.fetch_branches(core.git, repo, { integration }, "branch fetch")
  local rechecked_integration_sha = git_mechanics.remote_head(core.git, integration, "remote branch head", "unsafe remote branch head")
  if rechecked_integration_sha ~= integration_sha then
    core.log_cas_decision("sync_scan", "branch-sync", {
      state = "integration",
      version = rechecked_integration_sha,
    }, "sync", "push", "skip-foreign(head)", "integration head changed before push")
    return
  end

  local merge_head = trim_stdout(git_mechanics.run_required(git("github-devloop").git_head_sha(worktree, 30), "sync head"))
  if not require("devloop.pr_safety").is_safe_head_sha(merge_head) then
    error("github-devloop: unsafe branch sync merge head")
  end
  git_mechanics.run_required(git_mechanics.git_push_worktree_branch_update(core.git, worktree, integration, 120), "branch sync push")
  git_mechanics.fetch_branches(core.git, repo, { integration }, "branch fetch")
  local pushed_head = git_mechanics.remote_head(core.git, integration, "remote branch head", "unsafe remote branch head")
  if pushed_head ~= merge_head then
    error("github-devloop: branch sync push verification failed")
  end
  core.log_apply("sync_scan", "branch-sync", "synced", upstream_sha, {}, {})
end

local function converge_integration_to_upstream(repo, upstream, integration, upstream_sha, integration_sha)
  if config.write_mode(core) ~= "real" then
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

  devloop_base.assert_trusted_bot_configured()
  git_mechanics.fetch_branches(core.git, repo, { integration }, "branch fetch")
  local rechecked_integration_sha = git_mechanics.remote_head(core.git, integration, "remote branch head", "unsafe remote branch head")
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

  git_mechanics.run_required(git_mechanics.git_push_branch_force_with_lease(core.git, integration, upstream_sha, integration_sha, 120), "branch sync converge")
  git_mechanics.fetch_branches(core.git, repo, { integration }, "branch fetch")
  local pushed_head = git_mechanics.remote_head(core.git, integration, "remote branch head", "unsafe remote branch head")
  if pushed_head ~= upstream_sha then
    error("github-devloop: branch sync converge verification failed")
  end
  core.log_apply("sync_scan", "branch-sync", "converged", upstream_sha, {}, {})
end

local function fast_forward_sync(repo, upstream, integration, upstream_sha, integration_sha)
  local runtime = git_mechanics.runtime_root_with_exec(exec_sync)
  with_temp_worktree(runtime, repo, upstream, integration, integration_sha, function(worktree)
    git_mechanics.run_required(git_mechanics.git_fast_forward(core.git, worktree, upstream_sha, 120), "branch sync fast-forward")
    push_if_real(repo, upstream, integration, upstream_sha, integration_sha, worktree)
  end)
end

local function act(event)
  core.log_entry("sync_scan", event, "branch-sync", event and event.queue or "")
  local branches = config.branch_config(core)
  local cfg = config.devloop_config(core)
  local repo = require_repo(cfg.repo)

  if branches.integration == branches.upstream then
    core.log_cas_decision("sync_scan", "branch-sync", { state = "same-branch", version = branches.upstream }, "tick", "sync", "skip-idempotent(same-branch)", "integration branch equals upstream branch")
    return
  end

  with_lock(core.branch_sync_lock_key(repo, branches.upstream, branches.integration), function()
    git_mechanics.fetch_branches(core.git, repo, { branches.upstream, branches.integration }, "branch fetch")
    local upstream_sha = git_mechanics.remote_head(core.git, branches.upstream, "remote branch head", "unsafe remote branch head")
    local integration_sha = git_mechanics.remote_head(core.git, branches.integration, "remote branch head", "unsafe remote branch head")

    if git_mechanics.is_ancestor(core.git, upstream_sha, integration_sha, "ancestor check") then
      core.log_cas_decision("sync_scan", "branch-sync", { state = "synced", version = integration_sha }, "tick", "sync", "skip-idempotent(upstream-ancestor)", "upstream head is already contained in integration")
      return
    end
    if git_mechanics.is_ancestor(core.git, integration_sha, upstream_sha, "ancestor check") then
      fast_forward_sync(repo, branches.upstream, branches.integration, upstream_sha, integration_sha)
      return
    end
    if trees_equal(upstream_sha, integration_sha) then
      converge_integration_to_upstream(repo, branches.upstream, branches.integration, upstream_sha, integration_sha)
      return
    end

    local runtime = git_mechanics.runtime_root_with_exec(exec_sync)
    with_temp_worktree(runtime, repo, branches.upstream, branches.integration, integration_sha, function(worktree)
      local merge_result = git_mechanics.git_merge_no_ff(core.git, worktree, upstream_sha, 120)
      if merge_result.exit_code == 0 then
        write_sync_commit(worktree, runtime, repo, branches.upstream, branches.integration, upstream_sha, integration_sha, "clean")
        push_if_real(repo, branches.upstream, branches.integration, upstream_sha, integration_sha, worktree)
        return
      end

      local unmerged = core.git.unmerged_paths(worktree, 30)
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

return saga.department(spec, {
  done = done,
  act = act,
  wrap = core.wrap_pipeline_failure,
  name = "sync_scan",
})
