local git_mechanics = require("devloop.git_mechanics")
local devloop_base = require("devloop.base")
local error_facts = require("contract.error_facts")
local core = require("core")
local config = require("devloop.config")
local entity_lib = require("devloop.entity")
local m_claims = require("devloop.claims")
local m_facts = require("devloop.markers.facts")
local parsers_pr = require("devloop.parsers.pr")
local devloop_state = require("devloop.state")
local transition_version = require("contract.transition_version")
local merge_shared = require("forge.merge.shared")
local ports_seam = require("forge.ports")
local saga = require("workflow.saga")
local devloop_logging = require("devloop.logging")
local devloop_commands = require("devloop.commands")
local pr_commands = require("devloop.commands.prs")
local github_factory = require("devloop.github_factory")

local spec = {
  consumes = { "devloop_sync_conflict" },
  produces = { "github-proxy.github_issue_create_request" },
  stall_window = "10m",
}

local function trim_stdout(result)
  return tostring(result.stdout or ""):gsub("%s+$", "")
end

local function cleanup_worktree(git, worktree)
  if worktree == nil then
    return
  end
  local result = git.worktree_remove(worktree, 60)
  if result.exit_code ~= 0 then
    devloop_logging.log_line("warn", "sync_conflict", "branch-sync", "CLEANUP", {
      "worktree=" .. tostring(worktree),
      "reason=" .. error_facts.one_line(result.stderr or ""),
    })
  end
end

local function with_temp_worktree(git, conflict, fn)
  local runtime = git_mechanics.runtime_root_with_exec(exec_sync)
  local worktree = core.branch_sync_worktree_path(
    runtime,
    conflict.repo,
    conflict.upstream_branch,
    conflict.integration_branch,
    conflict.integration_sha
  )
  local plan = git.git_worktree_add_detached_plan(worktree, conflict.integration_sha)
  git_mechanics.run_required(exec_sync({ cmd = devloop_commands.mkdir_p_cmd(plan.parent_dir), timeout = 30 }), "worktree parent directory setup")
  git_mechanics.run_required(git.git_worktree_add_detached(plan.worktree, plan.sha, 60), "worktree add")

  local ok, result = pcall(fn, worktree, runtime)
  cleanup_worktree(git, worktree)
  if not ok then
    error(result)
  end
  return result
end

local function require_clean_resolution(git, worktree)
  local unmerged = git_mechanics.run_required(git.unmerged_paths(worktree, 30), "unmerged path check")
  if tostring(unmerged.stdout or "") ~= "" then
    return false, tostring(unmerged.stdout or "")
  end
  git_mechanics.run_required(git_mechanics.git_diff_check(git, worktree, 30), "diff check")
  git_mechanics.run_required(git_mechanics.git_diff_cached_check(git, worktree, 30), "cached diff check")
  return true, ""
end

local function normalize_self_hash_manifests(git, worktree, original_unmerged_stdout)
  local paths = core.sync_conflict_self_hash_normalizer_paths(original_unmerged_stdout)
  if #paths == 0 then
    return
  end
  local current_unmerged = git_mechanics.run_required(
    git.unmerged_paths(worktree, 30),
    "unmerged path check before self-hash normalization"
  )
  if tostring(current_unmerged.stdout or "") ~= "" then
    return
  end
  for _, path in ipairs(paths) do
    local result = exec_argv({
      argv = core.sync_conflict_self_hash_normalizer_argv(worktree, path),
      timeout = 120,
    })
    if type(result) ~= "table" or result.exit_code ~= 0 then
      error("github-devloop: self-hash-normalizer-failed: "
        .. tostring(path)
        .. ": "
        .. error_facts.one_line(type(result) == "table" and result.stderr or "missing command result"))
    end
    devloop_logging.log_line("info", "sync_conflict", "branch-sync", "NORMALIZE", {
      "path=" .. tostring(path),
      "reason=self-hashed generated manifest",
    })
  end
end

local function raise_sync_conflict_escalation(conflict, fingerprint, attempt, reason, unmerged_stdout)
  local request = core.build_sync_conflict_escalation_request(
    conflict,
    fingerprint,
    attempt,
    reason,
    unmerged_stdout
  )
  devloop_logging.log_raise("sync_conflict", "branch-sync", "github-proxy.github_issue_create_request", request)
  devloop_logging.log_error_fact("error", "sync_conflict", "branch-sync", "SYNC_CONFLICT_TERMINAL", "sync-conflict-unresolved", "devloop_sync_conflict", reason, {
    source_ref = conflict.source_ref,
    attempt = attempt,
    terminal = true,
  })
end

local function commit_resolution(git, worktree, runtime, conflict)
  git_mechanics.run_required(git.add_all(worktree, 30), "stage conflict resolution")
  local unmerged = git_mechanics.run_required(git.unmerged_paths(worktree, 30), "unmerged path check before commit")
  if tostring(unmerged.stdout or "") ~= "" then
    error("github-devloop: sync-conflict-unresolved: sync conflict remains unresolved before commit")
  end
  git_mechanics.run_required(git_mechanics.git_diff_cached_check(git, worktree, 30), "cached diff check before commit")
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
  git_mechanics.run_required(git.commit_message_file(worktree, message_file, 60), "sync commit")
end

local function push_if_real(git, conflict, worktree)
  if config.write_mode() ~= "real" then
    devloop_logging.log_line("info", "sync_conflict", "branch-sync", "OUTBOUND", {
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
  git_mechanics.fetch_branches(git, conflict.repo, { conflict.integration_branch }, "branch fetch")
  local rechecked_integration_sha = git_mechanics.remote_head(git, conflict.integration_branch, "remote branch head", "unsafe remote branch head")
  if rechecked_integration_sha ~= conflict.integration_sha then
    devloop_logging.log_cas_decision("sync_conflict", "branch-sync", {
      state = "integration",
      version = rechecked_integration_sha,
    }, "resolved", "push", "skip-foreign(head)", "integration head changed before resolved push")
    return
  end

  local merge_head = trim_stdout(git_mechanics.run_required(git.git_head_sha(worktree, 30), "resolved sync head"))
  if not require("devloop.pr_safety").is_safe_head_sha(merge_head) then
    error("github-devloop: unsafe-head-sha: unsafe resolved branch sync head")
  end
  git_mechanics.run_required(git_mechanics.git_push_worktree_branch_update(git, worktree, conflict.integration_branch, 120), "resolved branch sync push")
  git_mechanics.fetch_branches(git, conflict.repo, { conflict.integration_branch }, "branch fetch")
  local pushed_head = git_mechanics.remote_head(git, conflict.integration_branch, "remote branch head", "unsafe remote branch head")
  if pushed_head ~= merge_head then
    error("github-devloop: push-verification-mismatch: resolved branch sync push verification failed")
  end
  devloop_logging.log_apply("sync_conflict", "branch-sync", "synced", conflict.upstream_sha, {}, {})
end

local function decline_pr_recovery(conflict, origin, outcome, reason)
  devloop_logging.log_cas_decision("sync_conflict", origin and origin.proposal_id or "pr-freshness", {
    state = "conflict",
    version = conflict.integration_sha,
  }, "conflict", "closed-unmerged", outcome, reason)
  return false
end

local function read_pr_freshness(github, repo, pr_number, context)
  local viewed = git_mechanics.run_required(
    pr_commands.gh_pr_view_freshness(repo, pr_number, 30, github),
    context
  )
  local pr = parsers_pr.parse_pr_view_merge(viewed.stdout)
  pr.number = tonumber(pr_number)
  return pr
end

local function original_generation_branch(origin)
  local root_version = transition_version.strip_trailing_reimplement(origin.impl_version)
  return devloop_base.implement_branch(origin.repo, origin.issue_number, root_version)
end

local function validate_open_original_pr(conflict, source_repo, pr, origin, branches)
  if origin == nil or origin.pr_native == true or origin.repo ~= source_repo then
    return "fail-closed(pr-origin)", "PR lacks a trusted managed issue origin"
  end
  if origin.branch ~= original_generation_branch(origin) then
    return "fail-closed(replacement-generation)", "only a PR on the deterministic original implementation branch may be abandoned automatically"
  end
  if not merge_shared.is_same_repo_pr_head(pr, source_repo) then
    return "fail-closed(cross-repo)", "PR head repository is not the managed repository"
  end
  if pr.is_draft then
    return "fail-closed(draft)", "draft PR is outside managed freshness recovery scope"
  end
  if conflict.upstream_branch ~= branches.integration
    or origin.base_branch ~= branches.integration
    or pr.base_ref_name ~= branches.integration then
    return "fail-closed(base-branch)", "PR is not based on the configured integration branch"
  end
  if origin.branch ~= pr.head_ref_name
    or pr.head_ref_name ~= conflict.integration_branch then
    return "fail-closed(head-branch)", "PR head branch no longer matches the exhausted conflict"
  end
  if pr.head_sha ~= conflict.integration_sha then
    return "fail-closed(head-sha)", "PR head changed after the exhausted conflict was observed"
  end
  return nil, nil
end

local function read_matching_parent(github, source_repo, pr_number, origin)
  local parent = github.read_issue(entity_lib.issue_source_ref(origin.repo, origin.issue_number), {
    consumer = "sync_conflict",
    force_fresh = true,
  })
  local delegation = m_facts.pr_delegation_fact(parent.comments, origin.proposal_id, origin.impl_version)
  local delegated_repo, delegated_pr = nil, nil
  if delegation ~= nil then
    delegated_repo, delegated_pr = entity_lib.parse_pr_proposal_id(delegation.pr_proposal_id)
  end
  if tostring(parent.state or ""):upper() ~= "OPEN"
    or not devloop_state.is_current_state(parent.comments, origin.proposal_id, "awaiting-pr", origin.impl_version)
    or delegation == nil
    or delegated_repo ~= source_repo
    or tonumber(delegated_pr) ~= tonumber(pr_number)
    or tonumber(delegation.pr_number) ~= tonumber(pr_number) then
    return nil, "fail-closed(parent-delegation)", "parent no longer currently awaits this exact managed PR generation"
  end
  if m_claims.issue_claim_state(parent.assignees, m_claims.claim_owner(), parent.labels) ~= "self" then
    return nil, "fail-closed(parent-claim)", "parent issue is not held by the current self-only claim"
  end
  return parent, nil, nil
end

local function recover_exhausted_pr_freshness(github, git, conflict)
  local source_repo, pr_number = devloop_base.parse_pr_source_ref(conflict.source_ref)
  if source_repo == nil then
    return false
  end

  devloop_base.assert_trusted_bot_configured()
  local pr = read_pr_freshness(github, source_repo, pr_number, "PR freshness terminal re-read")
  local pr_state = tostring(pr.state or ""):upper()
  if pr_state == "CLOSED" or pr_state == "MERGED" then
    devloop_logging.log_cas_decision("sync_conflict", "pr-freshness", {
      state = pr_state:lower(),
      version = pr.head_sha,
    }, "conflict", "closed-unmerged", "skip-idempotent(pr-already-closed)", "exhausted PR is already closed; external observation owns terminal propagation")
    return true
  end
  if pr_state ~= "OPEN" then
    return decline_pr_recovery(conflict, nil, "fail-closed(pr-state)", "PR state is neither OPEN nor an explicit terminal state")
  end

  local origin = m_facts.pr_origin_fact(pr.comments)
  local branches = config.branch_config()
  if source_repo ~= conflict.repo then
    return decline_pr_recovery(conflict, origin, "fail-closed(source-repo)", "PR source_ref repository does not match the conflict repository")
  end
  local outcome, reason = validate_open_original_pr(conflict, source_repo, pr, origin, branches)
  if outcome ~= nil then
    return decline_pr_recovery(conflict, origin, outcome, reason)
  end

  local parent, parent_outcome, parent_reason = read_matching_parent(github, source_repo, pr_number, origin)
  if parent == nil then
    return decline_pr_recovery(conflict, origin, parent_outcome, parent_reason)
  end

  if config.write_mode() ~= "real" then
    devloop_logging.log_line("info", "sync_conflict", origin.proposal_id, "OUTBOUND", {
      "mode=dry-run",
      "repo=" .. tostring(source_repo),
      "pr=" .. tostring(pr_number),
      "head_sha=" .. tostring(pr.head_sha),
      "reason=would close exhausted original PR; external observation will drive replacement in real write mode",
    })
    return true
  end

  local current_pr = read_pr_freshness(github, source_repo, pr_number, "PR freshness final close re-read")
  if tostring(current_pr.state or ""):upper() ~= "OPEN" then
    return decline_pr_recovery(conflict, origin, "fail-closed(final-pr-state)", "PR is no longer OPEN immediately before close")
  end
  local current_origin = m_facts.pr_origin_fact(current_pr.comments)
  local final_outcome, final_reason = validate_open_original_pr(
    conflict,
    source_repo,
    current_pr,
    current_origin,
    branches
  )
  if final_outcome ~= nil
    or current_origin.proposal_id ~= origin.proposal_id
    or current_origin.impl_version ~= origin.impl_version then
    return decline_pr_recovery(
      conflict,
      current_origin,
      final_outcome or "fail-closed(final-pr-origin)",
      final_reason or "PR managed origin changed immediately before close"
    )
  end
  local current_parent, final_parent_outcome, final_parent_reason = read_matching_parent(
    github,
    source_repo,
    pr_number,
    current_origin
  )
  if current_parent == nil then
    return decline_pr_recovery(conflict, current_origin, final_parent_outcome, final_parent_reason)
  end
  git_mechanics.fetch_branches(git, conflict.repo, { conflict.upstream_branch }, "PR freshness final integration fetch")
  local current_integration_sha = git_mechanics.remote_head(
    git,
    conflict.upstream_branch,
    "PR freshness final integration head",
    "unsafe PR freshness integration head"
  )
  if current_integration_sha ~= conflict.upstream_sha then
    return decline_pr_recovery(conflict, current_origin, "fail-closed(final-integration-head)", "integration head advanced immediately before close")
  end

  local close_result = pr_commands.gh_pr_close(source_repo, pr_number, 60, github)
  if close_result.exit_code ~= 0 then
    error("github-devloop: pr-freshness-close-failed: " .. tostring(close_result.stderr))
  end
  require("devloop.github_proxy_entity_view").invalidate_entity_after_write(source_repo, "pr", pr_number)
  devloop_logging.log_line("info", "sync_conflict", origin.proposal_id, "OUTBOUND", {
    "mode=real",
    "repo=" .. tostring(source_repo),
    "pr=" .. tostring(pr_number),
    "head_sha=" .. tostring(pr.head_sha),
    "reason=closed exhausted original PR; awaiting normal external closed-unmerged observation",
  })
  return true
end

local function conflict_lock_key(conflict)
  if devloop_base.parse_pr_source_ref(conflict.source_ref) ~= nil then
    return core.pr_freshness_lock_key(conflict.repo, conflict.integration_branch)
  end
  return core.branch_sync_lock_key(conflict.repo, conflict.upstream_branch, conflict.integration_branch)
end

local function done(_event)
  return false
end

local function act(event, ports)
  local conflict = event.payload or {}
  if not core.is_supported_sync_conflict(conflict) then
    devloop_logging.log_entry("sync_conflict", event, "branch-sync", devloop_logging.payload_field(conflict, "dedup_key"))
    devloop_logging.log_cas_decision("sync_conflict", "branch-sync", { state = nil, version = nil }, "conflict", "resolved", "skip-foreign(payload)", "unsupported sync conflict payload")
    return
  end
  devloop_logging.log_entry("sync_conflict", event, "branch-sync", conflict.dedup_key)

  with_lock(conflict_lock_key(conflict), function()
    local git = ports.git
    git_mechanics.fetch_branches(git, conflict.repo, { conflict.upstream_branch, conflict.integration_branch }, "branch fetch")
    local upstream_sha = git_mechanics.remote_head(git, conflict.upstream_branch, "remote branch head", "unsafe remote branch head")
    local integration_sha = git_mechanics.remote_head(git, conflict.integration_branch, "remote branch head", "unsafe remote branch head")
    if integration_sha ~= conflict.integration_sha then
      devloop_logging.log_cas_decision("sync_conflict", "branch-sync", { state = "integration", version = integration_sha }, "conflict", "resolved", "skip-stale(integration-head)", "integration head advanced after conflict event")
      return
    end
    if git_mechanics.is_ancestor(git, upstream_sha, integration_sha, "ancestor check") then
      devloop_logging.log_cas_decision("sync_conflict", "branch-sync", { state = "synced", version = integration_sha }, "conflict", "resolved", "skip-idempotent(upstream-ancestor)", "conflict resolved elsewhere")
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

    with_temp_worktree(git, active_conflict, function(worktree, runtime)
      local merge_result = git_mechanics.git_merge_no_ff(git, worktree, active_conflict.upstream_sha, 120)
      if merge_result.exit_code == 0 then
        error("github-devloop: sync-conflict-stale: sync conflict event replayed without merge conflict")
      end
      local unmerged = git_mechanics.run_required(git.unmerged_paths(worktree, 30), "unmerged path check")
      if tostring(unmerged.stdout or "") == "" then
        error("github-devloop: merge-conflict-state-missing: sync conflict merge failed without unmerged paths")
      end
      local active_fingerprint = core.sync_conflict_fingerprint(active_conflict, tostring(unmerged.stdout or ""))
      local prior_attempts = core.sync_conflict_attempt_count(active_conflict, active_fingerprint)
      if prior_attempts >= core.max_sync_conflict_attempts() then
        if not recover_exhausted_pr_freshness(ports.github, git, active_conflict) then
          raise_sync_conflict_escalation(
            active_conflict,
            active_fingerprint,
            prior_attempts,
            "sync conflict retry budget already exhausted before codex",
            tostring(unmerged.stdout or "")
          )
        end
        return
      end

      devloop_logging.log_codex_start("sync_conflict", "branch-sync", "sync-conflict")
      local result = spawn_codex_sync({
        prompt = core.build_sync_conflict_prompt(active_conflict),
        worktree = worktree,
      })
      if type(result) ~= "table" or result.exit_code ~= 0 then
        local stderr = type(result) == "table" and result.stderr or "nil result"
        devloop_logging.log_codex_result("sync_conflict", "branch-sync", "sync-conflict", result, nil, stderr, {
          queue = event.queue,
          source_ref = conflict.source_ref,
          terminal = false,
        })
        error("github-devloop: sync-conflict-codex-failed: sync conflict codex failed: " .. tostring(stderr))
      end
      normalize_self_hash_manifests(git, worktree, tostring(unmerged.stdout or ""))
      local resolved, remaining_unmerged = require_clean_resolution(git, worktree)
      if not resolved then
        local fingerprint = core.sync_conflict_fingerprint(active_conflict, remaining_unmerged)
        local previous_attempts = core.sync_conflict_attempt_count(active_conflict, fingerprint)
        local attempt = previous_attempts + 1
        core.record_sync_conflict_attempt(active_conflict, fingerprint, attempt)
        local reason = "sync conflict remains unresolved after codex completed"
        devloop_logging.log_codex_result("sync_conflict", "branch-sync", "sync-conflict", result, nil, reason, {
          queue = event.queue,
          source_ref = conflict.source_ref,
          attempt = attempt,
          terminal = attempt >= core.max_sync_conflict_attempts(),
          error_class = "sync-conflict-unresolved",
        })
        if attempt >= core.max_sync_conflict_attempts() then
          if not recover_exhausted_pr_freshness(ports.github, git, active_conflict) then
            raise_sync_conflict_escalation(active_conflict, fingerprint, attempt, reason, remaining_unmerged)
          end
          return
        end
        error("github-devloop: sync-conflict-unresolved: " .. reason)
      end
      devloop_logging.log_codex_result("sync_conflict", "branch-sync", "sync-conflict", result, "result=completed", nil)
      commit_resolution(git, worktree, runtime, active_conflict)
      push_if_real(git, active_conflict, worktree)
    end)
  end)
end

local function make_department(ports)
  local previous_pipeline = _G.pipeline
  local department = saga.department(spec, {
    done = done,
    act = function(event) return act(event, ports) end,
    wrap = devloop_logging.wrap_pipeline_failure,
    name = "sync_conflict",
  })
  department.pipeline = _G.pipeline
  _G.pipeline = previous_pipeline
  return department
end

local M = ports_seam.install(make_department, github_factory.github_options(exec_sync))
_G.pipeline = M.pipeline

return M
