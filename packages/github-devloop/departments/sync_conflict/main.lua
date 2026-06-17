local core = require("core")

local M = {}

M.spec = {
  consumes = { "devloop_sync_conflict" },
  produces = { "github-proxy.github_issue_create_request" },
  stall_window = "10m",
}

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
  local plan = core.git_worktree_add_detached_plan(worktree, conflict.integration_sha)
  run_required(exec_sync({ cmd = core.mkdir_p_cmd(plan.parent_dir), timeout = 30 }), "worktree parent directory setup")
  require_git_ok(core.git_worktree_add_detached(plan.worktree, plan.sha, 60), "worktree add")

  local ok, result = pcall(fn, worktree, runtime)
  cleanup_worktree(worktree)
  if not ok then
    error(result)
  end
  return result
end

local function require_clean_resolution(worktree)
  local unmerged = require_git_ok(core.git_unmerged_paths(worktree, 30), "unmerged path check")
  if tostring(unmerged.stdout or "") ~= "" then
    return false, tostring(unmerged.stdout or "")
  end
  require_git_ok(core.git_diff_check(worktree, 30), "diff check")
  require_git_ok(core.git_diff_cached_check(worktree, 30), "cached diff check")
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
  run_required(core.git_add_all(worktree, 30), "stage conflict resolution")
  local unmerged = require_git_ok(core.git_unmerged_paths(worktree, 30), "unmerged path check before commit")
  if tostring(unmerged.stdout or "") ~= "" then
    error("github-devloop: sync conflict remains unresolved before commit")
  end
  require_git_ok(core.git_diff_cached_check(worktree, 30), "cached diff check before commit")
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
  require_git_ok(core.git_commit_message_file(worktree, message_file, 60), "sync commit")
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
  fetch_branches(conflict.repo, { conflict.integration_branch })
  local rechecked_integration_sha = remote_head(conflict.integration_branch)
  if rechecked_integration_sha ~= conflict.integration_sha then
    core.log_cas_decision("sync_conflict", "branch-sync", {
      state = "integration",
      version = rechecked_integration_sha,
    }, "resolved", "push", "skip-foreign(head)", "integration head changed before resolved push")
    return
  end

  local merge_head = trim_stdout(run_required(core.git_head_sha(worktree, 30), "resolved sync head"))
  if not core.is_safe_head_sha(merge_head) then
    error("github-devloop: unsafe resolved branch sync head")
  end
  require_git_ok(core.git_push_worktree_branch_update(worktree, conflict.integration_branch, 120), "resolved branch sync push")
  fetch_branches(conflict.repo, { conflict.integration_branch })
  local pushed_head = remote_head(conflict.integration_branch)
  if pushed_head ~= merge_head then
    error("github-devloop: resolved branch sync push verification failed")
  end
  core.log_apply("sync_conflict", "branch-sync", "synced", conflict.upstream_sha, {}, {})
end

function pipeline(event)
  local conflict = event.payload or {}
  if not core.is_supported_sync_conflict(conflict) then
    core.log_entry("sync_conflict", event, "branch-sync", core.payload_field(conflict, "dedup_key"))
    core.log_cas_decision("sync_conflict", "branch-sync", { state = nil, version = nil }, "conflict", "resolved", "skip-foreign(payload)", "unsupported sync conflict payload")
    return
  end
  core.log_entry("sync_conflict", event, "branch-sync", conflict.dedup_key)

  with_lock(core.branch_sync_lock_key(conflict.repo, conflict.upstream_branch, conflict.integration_branch), function()
    fetch_branches(conflict.repo, { conflict.upstream_branch, conflict.integration_branch })
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
      local merge_result = core.git_merge_no_ff(worktree, active_conflict.upstream_sha, 120)
      if merge_result.exit_code == 0 then
        error("github-devloop: sync conflict event replayed without merge conflict")
      end
      local unmerged = require_git_ok(core.git_unmerged_paths(worktree, 30), "unmerged path check")
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

pipeline = core.wrap_pipeline_failure("sync_conflict", pipeline)

return M
