local git_mechanics = require("devloop.git_mechanics")
local devloop_base = require("devloop.base")
local error_facts = require("contract.error_facts")
local core = require("core")
local config = require("devloop.config")
local git_adapter = require("forge.git")
local saga = require("workflow.saga")

local spec = {
  consumes = { "devloop_sync_conflict" },
  produces = { "github-proxy.github_issue_create_request" },
  stall_window = "10m",
}

local git = git_adapter.production_handle

local function trim_stdout(result)
  return tostring(result.stdout or ""):gsub("%s+$", "")
end

local function cleanup_worktree(worktree)
  if worktree == nil then
    return
  end
  local result = core.git.worktree_remove(worktree, 60)
  if result.exit_code ~= 0 then
    core.log_line("warn", "sync_conflict", "branch-sync", "CLEANUP", {
      "worktree=" .. tostring(worktree),
      "reason=" .. error_facts.one_line(result.stderr or ""),
    })
  end
end

local function with_temp_worktree(conflict, fn)
  local runtime = git_mechanics.runtime_root_with_exec(exec_sync)
  local worktree = core.branch_sync_worktree_path(
    runtime,
    conflict.repo,
    conflict.upstream_branch,
    conflict.integration_branch,
    conflict.integration_sha
  )
  local plan = git("github-devloop").git_worktree_add_detached_plan(worktree, conflict.integration_sha)
  git_mechanics.run_required(exec_sync({ cmd = core.mkdir_p_cmd(plan.parent_dir), timeout = 30 }), "worktree parent directory setup")
  git_mechanics.run_required(git("github-devloop").git_worktree_add_detached(plan.worktree, plan.sha, 60), "worktree add")

  local ok, result = pcall(fn, worktree, runtime)
  cleanup_worktree(worktree)
  if not ok then
    error(result)
  end
  return result
end

local function require_clean_resolution(worktree)
  local unmerged = git_mechanics.run_required(core.git.unmerged_paths(worktree, 30), "unmerged path check")
  if tostring(unmerged.stdout or "") ~= "" then
    return false, tostring(unmerged.stdout or "")
  end
  git_mechanics.run_required(git_mechanics.git_diff_check(core.git, worktree, 30), "diff check")
  git_mechanics.run_required(git_mechanics.git_diff_cached_check(core.git, worktree, 30), "cached diff check")
  return true, ""
end

local function raise_sync_conflict_escalation(conflict, fingerprint, attempt, reason, unmerged_stdout)
  local request = core.build_sync_conflict_escalation_request(
    conflict,
    fingerprint,
    attempt,
    reason,
    unmerged_stdout
  )
  core.log_raise("sync_conflict", "branch-sync", "github-proxy.github_issue_create_request", request)
  core.log_error_fact("error", "sync_conflict", "branch-sync", "SYNC_CONFLICT_TERMINAL", "sync-conflict-unresolved", "devloop_sync_conflict", reason, {
    source_ref = conflict.source_ref,
    attempt = attempt,
    terminal = true,
  })
end

local function commit_resolution(worktree, runtime, conflict)
  git_mechanics.run_required(core.git_add_all(worktree, 30), "stage conflict resolution")
  local unmerged = git_mechanics.run_required(core.git.unmerged_paths(worktree, 30), "unmerged path check before commit")
  if tostring(unmerged.stdout or "") ~= "" then
    error("github-devloop: sync conflict remains unresolved before commit")
  end
  git_mechanics.run_required(git_mechanics.git_diff_cached_check(core.git, worktree, 30), "cached diff check before commit")
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
  git_mechanics.run_required(core.git.commit_message_file(worktree, message_file, 60), "sync commit")
end

local function push_if_real(conflict, worktree)
  if config.write_mode(core) ~= "real" then
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

  devloop_base.assert_trusted_bot_configured()
  git_mechanics.fetch_branches(core.git, conflict.repo, { conflict.integration_branch }, "branch fetch")
  local rechecked_integration_sha = git_mechanics.remote_head(core.git, conflict.integration_branch, "remote branch head", "unsafe remote branch head")
  if rechecked_integration_sha ~= conflict.integration_sha then
    core.log_cas_decision("sync_conflict", "branch-sync", {
      state = "integration",
      version = rechecked_integration_sha,
    }, "resolved", "push", "skip-foreign(head)", "integration head changed before resolved push")
    return
  end

  local merge_head = trim_stdout(git_mechanics.run_required(git("github-devloop").git_head_sha(worktree, 30), "resolved sync head"))
  if not require("devloop.pr_safety").is_safe_head_sha(merge_head) then
    error("github-devloop: unsafe resolved branch sync head")
  end
  git_mechanics.run_required(git_mechanics.git_push_worktree_branch_update(core.git, worktree, conflict.integration_branch, 120), "resolved branch sync push")
  git_mechanics.fetch_branches(core.git, conflict.repo, { conflict.integration_branch }, "branch fetch")
  local pushed_head = git_mechanics.remote_head(core.git, conflict.integration_branch, "remote branch head", "unsafe remote branch head")
  if pushed_head ~= merge_head then
    error("github-devloop: resolved branch sync push verification failed")
  end
  core.log_apply("sync_conflict", "branch-sync", "synced", conflict.upstream_sha, {}, {})
end

local function done(_event)
  return false
end

local function act(event)
  local conflict = event.payload or {}
  if not core.is_supported_sync_conflict(conflict) then
    core.log_entry("sync_conflict", event, "branch-sync", core.payload_field(conflict, "dedup_key"))
    core.log_cas_decision("sync_conflict", "branch-sync", { state = nil, version = nil }, "conflict", "resolved", "skip-foreign(payload)", "unsupported sync conflict payload")
    return
  end
  core.log_entry("sync_conflict", event, "branch-sync", conflict.dedup_key)

  with_lock(core.branch_sync_lock_key(conflict.repo, conflict.upstream_branch, conflict.integration_branch), function()
    git_mechanics.fetch_branches(core.git, conflict.repo, { conflict.upstream_branch, conflict.integration_branch }, "branch fetch")
    local upstream_sha = git_mechanics.remote_head(core.git, conflict.upstream_branch, "remote branch head", "unsafe remote branch head")
    local integration_sha = git_mechanics.remote_head(core.git, conflict.integration_branch, "remote branch head", "unsafe remote branch head")
    if integration_sha ~= conflict.integration_sha then
      core.log_cas_decision("sync_conflict", "branch-sync", { state = "integration", version = integration_sha }, "conflict", "resolved", "skip-stale(integration-head)", "integration head advanced after conflict event")
      return
    end
    if git_mechanics.is_ancestor(core.git, upstream_sha, integration_sha, "ancestor check") then
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
      local merge_result = git_mechanics.git_merge_no_ff(core.git, worktree, active_conflict.upstream_sha, 120)
      if merge_result.exit_code == 0 then
        error("github-devloop: sync conflict event replayed without merge conflict")
      end
      local unmerged = git_mechanics.run_required(core.git.unmerged_paths(worktree, 30), "unmerged path check")
      if tostring(unmerged.stdout or "") == "" then
        error("github-devloop: sync conflict merge failed without unmerged paths")
      end
      local active_fingerprint = core.sync_conflict_fingerprint(active_conflict, tostring(unmerged.stdout or ""))
      local prior_attempts = core.sync_conflict_attempt_count(active_conflict, active_fingerprint)
      if prior_attempts >= core.max_sync_conflict_attempts() then
        raise_sync_conflict_escalation(
          active_conflict,
          active_fingerprint,
          prior_attempts,
          "sync conflict retry budget already exhausted before codex",
          tostring(unmerged.stdout or "")
        )
        return
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
      local resolved, remaining_unmerged = require_clean_resolution(worktree)
      if not resolved then
        local fingerprint = core.sync_conflict_fingerprint(active_conflict, remaining_unmerged)
        local previous_attempts = core.sync_conflict_attempt_count(active_conflict, fingerprint)
        local attempt = previous_attempts + 1
        core.record_sync_conflict_attempt(active_conflict, fingerprint, attempt)
        local reason = "sync conflict remains unresolved after codex completed"
        core.log_codex_result("sync_conflict", "branch-sync", "sync-conflict", result, nil, reason, {
          queue = event.queue,
          source_ref = conflict.source_ref,
          attempt = attempt,
          terminal = attempt >= core.max_sync_conflict_attempts(),
          error_class = "sync-conflict-unresolved",
        })
        if attempt >= core.max_sync_conflict_attempts() then
          raise_sync_conflict_escalation(active_conflict, fingerprint, attempt, reason, remaining_unmerged)
          return
        end
        error("github-devloop: sync-conflict-unresolved: " .. reason)
      end
      core.log_codex_result("sync_conflict", "branch-sync", "sync-conflict", result, "result=completed", nil)
      commit_resolution(worktree, runtime, active_conflict)
      push_if_real(active_conflict, worktree)
    end)
  end)
end

return saga.department(spec, {
  done = done,
  act = act,
  wrap = core.wrap_pipeline_failure,
  name = "sync_conflict",
})
