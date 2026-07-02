local devloop_base = require("devloop.base")
local entity_lib = require("devloop.entity")
local m_claims = require("devloop.claims")
local requests_labels = require("devloop.requests.labels")
local requests_review = require("devloop.requests.review")
local parsers_pr = require("devloop.parsers.pr")
local parsers_issue = require("devloop.parsers.issue")
local core = require("core")
local git_adapter = require("forge.git")
local saga = require("workflow.saga")
local dispatch_live_run = require("devloop.dispatch_live_run")
local conflict_telemetry = require("devloop.conflict_telemetry")
local context_bundle = require("devloop.context_bundle")
local config = require("devloop.config")
local m_mq = require("devloop.merge_queue")

local payloads_builders = require("devloop.payloads.builders")
local v_fixing = require("devloop.validators.fixing")
local m_facts = require("devloop.markers.facts")
local spec = {
  consumes = { "devloop_fixing" },
  produces = {
    "github-proxy.github_issue_label_request",
    "github-proxy.github_pr_comment_request",
    "devloop_review_meta",
  },
  stall_window = "10m",
  retry = { max_attempts = 12, base = "5s", cap = "30s" },
}

local git = git_adapter.production_handle

local function fix_done(_event)
  return false
end

local function branch_worktree(repo, issue_number, version, branch)
  local runtime_result = exec_sync({ cmd = core.read_runtime_root_cmd(), timeout = 30 })
  if runtime_result.exit_code ~= 0 then
    error("github-devloop: FKST_RUNTIME_ROOT read failed: " .. tostring(runtime_result.stderr))
  end
  local runtime_root = runtime_result.stdout
  local worktree = devloop_base.implement_worktree_path(runtime_root, repo, issue_number, version)
  local list_result = core.git_worktree_list(30)
  if list_result.exit_code ~= 0 then
    error("github-devloop: git worktree list failed: " .. tostring(list_result.stderr))
  end
  local existing = core.find_worktree_for_branch(list_result.stdout, branch)
  if existing ~= nil then
    local dir_result = exec_sync({ cmd = core.path_is_directory_cmd(existing), timeout = 30 })
    if dir_result.exit_code ~= 0 and dir_result.exit_code ~= 1 then
      error("github-devloop: git worktree path check failed: " .. tostring(dir_result.stderr))
    end
    if dir_result.exit_code == 0 and devloop_base.path_under_runtime_root(runtime_root, existing) then
      return existing
    end
    if dir_result.exit_code == 1 then
      local prune_result = core.git_worktree_prune(60)
      if prune_result.exit_code ~= 0 then
        error("github-devloop: git worktree prune failed: " .. tostring(prune_result.stderr))
      end
    else
      local remove_result = core.git.worktree_remove(existing, 60)
      if remove_result.exit_code ~= 0 then
        error("github-devloop: git worktree remove failed: " .. tostring(remove_result.stderr))
      end
    end
  end

  local fetch_result = core.git_fetch_branch("origin", branch, 60)
  if fetch_result.exit_code ~= 0 then
    error("github-devloop: git PR head branch fetch failed: " .. tostring(fetch_result.stderr))
  end
  local add_result = core.git_worktree_add_remote_branch(worktree, "origin", branch, existing ~= nil, 60)
  if add_result.exit_code ~= 0 then
    error("github-devloop: git worktree add failed: " .. tostring(add_result.stderr))
  end
  return worktree
end

local function fetch_expected_pr_merge_product(pr_number, expected_baseline_sha)
  if expected_baseline_sha == nil then
    return nil
  end
  local fetch_result = core.git_fetch_pr_merge_ref("origin", pr_number, 60)
  if fetch_result.exit_code ~= 0 then
    error("github-devloop: git PR merge ref fetch failed: " .. tostring(fetch_result.stderr))
  end
  local head_result = core.git_fetch_head_commit(30)
  if head_result.exit_code ~= 0 then
    error("github-devloop: git PR merge ref head failed: " .. tostring(head_result.stderr))
  end
  local merge_product_sha = tostring(head_result.stdout or ""):gsub("%s+$", "")
  if merge_product_sha ~= expected_baseline_sha then
    error("github-devloop: PR merge ref head does not match merge-gate baseline")
  end
  return merge_product_sha
end

local function merge_integration_for_fix(worktree, pr_number, integration_branch, expected_baseline_sha, merge_gate_reason)
  if core.merge_gate_reason_requires_pr_merge_product(merge_gate_reason) then
    fetch_expected_pr_merge_product(pr_number, expected_baseline_sha)
  end
  local base_head = expected_baseline_sha
  if base_head == nil then
    local fetch_result = core.git_fetch_branch("origin", integration_branch, 60)
    if fetch_result.exit_code ~= 0 then
      error("github-devloop: git integration branch fetch failed: " .. tostring(fetch_result.stderr))
    end
    local base_result = core.git_remote_branch_head("origin", integration_branch, 30)
    if base_result.exit_code ~= 0 then
      error("github-devloop: git integration branch head failed: " .. tostring(base_result.stderr))
    end
    base_head = tostring(base_result.stdout or ""):gsub("%s+$", "")
  end
  if not require("devloop.pr_safety").is_safe_head_sha(base_head) then
    error("github-devloop: unsafe integration head")
  end
  local result = {
    target_branch = integration_branch,
    target_sha = base_head,
    conflicted = false,
    unmerged_paths = "",
  }
  local merge_result = core.git_worktree_merge_no_edit(worktree, base_head, 120)
  if merge_result.exit_code ~= 0 then
    local unmerged_result = core.git.unmerged_paths(worktree, 30)
    if unmerged_result.exit_code ~= 0 then
      error("github-devloop: git unmerged path check failed: " .. tostring(unmerged_result.stderr))
    end
    if tostring(unmerged_result.stdout or "") == "" then
      error("github-devloop: git integration merge failed: " .. tostring(merge_result.stderr))
    end
    result.conflicted = true
    result.unmerged_paths = tostring(unmerged_result.stdout or "")
    core.log_line("info", "fix", "merge-target", "MERGE_SKEW", {
      "integration_branch=" .. tostring(integration_branch),
      "integration_sha=" .. tostring(base_head),
      "reason=integration merge requires codex conflict resolution",
    })
  end
  return result
end

local function merge_result_context(target_branch, target_sha)
  return {
    target_branch = target_branch,
    target_sha = target_sha,
    conflicted = false,
    unmerged_paths = "",
  }
end

local function append_unmerged_paths(left, right)
  if tostring(left or "") == "" then
    return tostring(right or "")
  end
  if tostring(right or "") == "" then
    return tostring(left or "")
  end
  return tostring(left) .. "\n" .. tostring(right)
end

local function merge_sha_for_fix(worktree, sha, context, log_values)
  local merge_result = core.git_worktree_merge_no_edit(worktree, sha, 120)
  if merge_result.exit_code == 0 then
    return context
  end
  local unmerged_result = core.git.unmerged_paths(worktree, 30)
  if unmerged_result.exit_code ~= 0 then
    error("github-devloop: git unmerged path check failed: " .. tostring(unmerged_result.stderr))
  end
  if tostring(unmerged_result.stdout or "") == "" then
    error("github-devloop: git target merge failed: " .. tostring(merge_result.stderr))
  end
  context.conflicted = true
  context.unmerged_paths = append_unmerged_paths(context.unmerged_paths, unmerged_result.stdout)
  core.log_line("info", "fix", "merge-target", "MERGE_SKEW", log_values)
  return context
end

local function fetch_verified_pr_head(pr_number, expected_head_sha)
  local fetch_result = core.git_fetch_pr_head_ref("origin", pr_number, 60)
  if fetch_result.exit_code ~= 0 then
    error("github-devloop: git PR head ref fetch failed: " .. tostring(fetch_result.stderr))
  end
  local head_result = core.git_fetch_head_commit(30)
  if head_result.exit_code ~= 0 then
    error("github-devloop: git PR head ref head failed: " .. tostring(head_result.stderr))
  end
  local head_sha = tostring(head_result.stdout or ""):gsub("%s+$", "")
  if head_sha ~= tostring(expected_head_sha) then
    error("github-devloop: PR head ref does not match merge queue predecessor")
  end
  return head_sha
end

local function current_predecessors_for_fix(repo, integration_branch, fix, current_pr)
  local predecessors, reason = m_mq.merge_queue_predecessors(core, repo, integration_branch, {
    pr_number = fix.pr_number,
    pr = current_pr,
  })
  if predecessors == nil then
    return nil, reason
  end
  return predecessors, m_mq.merge_queue_predecessor_set(core, predecessors)
end

local function merge_speculative_predecessors_for_fix(worktree, repo, integration_branch, fix, current_pr)
  if fix.predecessor_set == nil then
    return nil, "not-speculative"
  end
  local predecessors, current_set = current_predecessors_for_fix(repo, integration_branch, fix, current_pr)
  if predecessors == nil then
    return nil, current_set
  end
  if tostring(current_set) ~= tostring(fix.predecessor_set) then
    return nil, "predecessor-set-mismatch", current_set
  end
  if #predecessors == 0 then
    return nil, "not-speculative"
  end
  local context = merge_result_context("speculative:" .. integration_branch, current_set)
  for _, predecessor in ipairs(predecessors) do
    if not require("devloop.pr_safety").is_safe_head_sha(predecessor.head_sha) then
      error("github-devloop: unsafe speculative predecessor head")
    end
    local predecessor_head = fetch_verified_pr_head(predecessor.pr_number, predecessor.head_sha)
    context = merge_sha_for_fix(worktree, predecessor_head, context, {
      "integration_branch=" .. tostring(integration_branch),
      "predecessor_pr=" .. tostring(predecessor.pr_number),
      "predecessor_head=" .. tostring(predecessor_head),
      "reason=speculative predecessor merge requires codex conflict resolution",
    })
  end
  return context, "ok"
end

local function assert_no_unmerged_paths(worktree)
  local unmerged_result = core.git.unmerged_paths(worktree, 30)
  if unmerged_result.exit_code ~= 0 then
    error("github-devloop: git unmerged path check failed: " .. tostring(unmerged_result.stderr))
  end
  if tostring(unmerged_result.stdout or "") ~= "" then
    error("github-devloop: fix left target merge conflicts unresolved")
  end
end

local function assert_no_conflict_markers(worktree)
  local markers_result = core.git.conflict_markers(worktree, 30)
  if markers_result.exit_code == 1 then
    return
  end
  if markers_result.exit_code == 0 then
    error("github-devloop: fix left conflict markers unresolved")
  end
  error("github-devloop: git conflict marker check failed: " .. tostring(markers_result.stderr))
end

local function bounded_fix_summary(value)
  local text = tostring(value or ""):gsub("%s+", " "):gsub("^%s+", ""):gsub("%s+$", "")
  if #text > 600 then
    text = text:sub(1, 600)
  end
  return text
end

local function raise_review_meta(repo, issue_number, fix, reason, detail)
  local comment_request = core.build_fix_review_meta_comment_request(repo, issue_number, fix, reason, detail)
  local label_request = core.build_fix_review_meta_label_request(repo, issue_number, fix, reason)
  local add_labels, remove_labels = core.state_label_changes("review-meta")
  core.log_apply("fix", fix.proposal_id, "review-meta", fix.version, { add = add_labels, remove = remove_labels }, {
    "github-proxy.github_pr_comment_request",
    "github-proxy.github_issue_label_request",
    "devloop_review_meta",
  })
  core.log_raise("fix", fix.proposal_id, "github-proxy.github_pr_comment_request", comment_request)
  if issue_number ~= nil then
    core.log_raise("fix", fix.proposal_id, "github-proxy.github_issue_label_request", label_request)
  end
  core.log_raise("fix", fix.proposal_id, "devloop_review_meta", {
    schema = "github-devloop.review-meta.v1",
    proposal_id = fix.proposal_id,
    review_proposal_id = fix.review_proposal_id,
    review_dedup_key = fix.review_dedup_key,
    version = fix.version,
    pr_number = fix.pr_number,
    n = 0,
    dedup_key = fix.dedup_key,
    source_ref = fix.source_ref,
  })
end

local function raise_reviewing(repo, issue_number, fix, old_head_sha, new_head_sha, reason, summary)
  requests_review.raise_fix_reviewing(core, {
    dept = "fix",
    repo = repo,
    issue_number = issue_number,
    fix = fix,
    old_head_sha = old_head_sha,
    new_head_sha = new_head_sha,
    reason = reason,
    fix_summary = bounded_fix_summary(summary),
    clear_fix_summary = true,
  })
end

local function raise_stale_speculation_refix(repo, issue_number, fix, current_state, current_predecessor_set, reason)
  local next_version = core.next_fix_version(fix.version)
  local merge_ready = {
    proposal_id = fix.proposal_id,
    pr_number = fix.pr_number,
    version = core._strip_latest_fix_version_suffix(fix.version),
    review_proposal_id = fix.review_proposal_id,
    review_dedup_key = fix.review_dedup_key,
    reviewed_head_sha = fix.reviewed_head_sha,
    dedup_key = fix.dedup_key,
  }
  local comment_request = requests_review.build_merge_gate_fix_comment_request(core,
    repo,
    issue_number,
    merge_ready,
    next_version,
    fix.gate_failure_excerpt or fix.blocking_gap or reason,
    fix.gate_baseline_sha,
    fix.source_ref,
    current_predecessor_set,
    {
      blocking_gap = fix.blocking_gap,
      gate_failure_excerpt = fix.gate_failure_excerpt,
      preserve_nil_gate_failure_excerpt = true,
    }
  )
  local label_request = issue_number ~= nil and requests_labels.build_state_label_request(core,
    repo,
    issue_number,
    "fixing",
    fix.dedup_key .. "/label/refix/" .. tostring(core.version_fix_round(next_version)),
    entity_lib.issue_source_ref(repo, issue_number)
  ) or nil
  local add_labels, remove_labels = core.state_label_changes("fixing")
  core.log_cas_decision("fix", fix.proposal_id, current_state, "fixing", "fixing", "applied", reason)
  local raised = {
    "github-proxy.github_pr_comment_request",
  }
  if label_request ~= nil then
    table.insert(raised, "github-proxy.github_issue_label_request")
  end
  core.log_apply("fix", fix.proposal_id, "fixing", next_version, { add = add_labels, remove = remove_labels }, raised)
  core.log_raise("fix", fix.proposal_id, "github-proxy.github_pr_comment_request", comment_request)
  if label_request ~= nil then
    core.log_raise("fix", fix.proposal_id, "github-proxy.github_issue_label_request", label_request)
  end
end

local function assert_fix_write_gate(fix, repo, issue_number)
  local write_enabled = config.write_mode(core) == "real"
  if write_enabled then
    return true
  end
  core.log_line("info", "fix", fix.proposal_id, "OUTBOUND", {
    "mode=dry-run",
    "repo=" .. tostring(repo),
    "issue=" .. tostring(issue_number),
    "pr=" .. tostring(fix.pr_number),
    "reason=PR fix requires FKST_GITHUB_WRITE=1 before codex",
  })
  return false
end

local function branch_head_if_ahead(base_head_sha, branch)
  local ahead_result = core.git_branch_ahead_count(base_head_sha, branch, 30)
  if ahead_result.exit_code ~= 0 then
    error("github-devloop: git branch ahead check failed: " .. tostring(ahead_result.stderr))
  end
  local ahead_count = tonumber(tostring(ahead_result.stdout or ""):match("%d+"))
  if ahead_count == nil or ahead_count <= 0 then
    return nil
  end
  local head_result = core.git_branch_head(branch, 30)
  if head_result.exit_code ~= 0 then
    error("github-devloop: git branch head check failed: " .. tostring(head_result.stderr))
  end
  local branch_head_sha = tostring(head_result.stdout or ""):gsub("%s+$", "")
  if not require("devloop.pr_safety").is_safe_head_sha(branch_head_sha) then
    error("github-devloop: unsafe deterministic branch head sha")
  end
  if branch_head_sha == base_head_sha then
    return nil
  end
  return branch_head_sha
end

local function run_fix_attempt(plan)
  local worktree = branch_worktree(plan.repo, plan.issue_number, plan.fix.version, plan.branch)
  local merge_context, speculative_reason, speculative_current_set = merge_speculative_predecessors_for_fix(
    worktree,
    plan.repo,
    plan.branches.integration,
    plan.fix,
    plan.current_pr
  )
  if merge_context == nil and speculative_reason ~= "not-speculative" then
    if speculative_reason == "predecessor-set-mismatch" then
      return {
        kind = "refix",
        current_predecessor_set = speculative_current_set or "none",
        reason = "speculative predecessor set changed",
      }
    end
    core.log_cas_decision("fix", plan.fix.proposal_id, plan.state, "fixing", "reviewing", "skip-stale(" .. tostring(speculative_reason) .. ")", "speculative predecessor set is no longer current")
    return nil
  end
  if merge_context == nil then
    merge_context = merge_integration_for_fix(
      worktree,
      plan.fix.pr_number,
      plan.branches.integration,
      plan.merge_gate_fact and plan.merge_gate_fact.gate_baseline_sha or nil,
      plan.merge_gate_fact and plan.merge_gate_fact.reason or nil
    )
  end
  if merge_context.conflicted then
    conflict_telemetry.log_conflict_files(core, "fix", plan.fix.proposal_id, plan.fix.pr_number, merge_context.unmerged_paths)
  end
  local codex_started_at = now()
  core.log_codex_start("fix", plan.fix.proposal_id, "fix")
  local content_fetch = context_bundle.context_fetch_from_bundle(core, {
    dept = "fix",
    repo = plan.repo,
    issue_number = plan.issue_number,
    pr_number = plan.fix.pr_number,
    proposal_id = plan.fix.proposal_id,
    version = plan.fix.dedup_key,
    tick = plan.event_ts,
  })
  local result = spawn_codex_sync({
    prompt = core.build_fix_prompt(plan.fix, plan.current_issue, plan.feedback_reason, plan.fix.framing, content_fetch, merge_context),
    worktree = worktree, role = "fix", proposal_id = plan.fix.proposal_id, dedup_key = plan.fix.version, timeout = 2 * 60 * 60,  -- 2h: fix loops code+test (#1481)
  })
  if type(result) ~= "table" or result.exit_code ~= 0 then
    local stderr = type(result) == "table" and result.stderr or "nil result"
    core.log_codex_result("fix", plan.fix.proposal_id, "fix", result, nil, stderr, {
      queue = plan.event_queue,
      source_ref = plan.fix.source_ref,
      terminal = false,
    })
    return {
      kind = "review-meta",
      reason = "codex-failed",
      detail = stderr,
      outcome = "failed: codex-failed",
      started_at = codex_started_at,
      finished_at = now(),
    }
  end
  core.log_codex_result("fix", plan.fix.proposal_id, "fix", result, "result=completed", nil)
  assert_no_unmerged_paths(worktree)
  assert_no_conflict_markers(worktree)

  local status = core.git_status(worktree, 30)
  if status.exit_code ~= 0 then
    error("github-devloop: git status failed: " .. tostring(status.stderr))
  end
  if tostring(status.stdout or "") == "" then
    local existing_head_sha = branch_head_if_ahead(plan.fix.reviewed_head_sha, plan.branch)
    if existing_head_sha ~= nil then
      core.log_codex_result("fix", plan.fix.proposal_id, "fix", result, "result=reusing-existing-head", nil)
      return {
        kind = "reviewing",
        old_head_sha = plan.fix.reviewed_head_sha,
        new_head_sha = existing_head_sha,
        reason = "existing fix commit pushed and PR head verified",
        summary = result.stdout or result.stderr,
        outcome = "completed: existing head pushed",
        started_at = codex_started_at,
        finished_at = now(),
      }
    end
    core.log_codex_result("fix", plan.fix.proposal_id, "fix", result, nil, "no-changes", {
      queue = plan.event_queue,
      source_ref = plan.fix.source_ref,
      terminal = false,
    })
    return {
      kind = "review-meta",
      reason = "no-fix",
      detail = result.stdout or result.stderr,
      outcome = "escalated: no-fix",
      started_at = codex_started_at,
      finished_at = now(),
    }
  end

  local add_result = core.git_add_all(worktree, 30)
  if add_result.exit_code ~= 0 then
    error("github-devloop: git add failed: " .. tostring(add_result.stderr))
  end
  local commit_result = core.git_commit(worktree, payloads_builders.fix_commit_subject(core,
      plan.issue_number,
      require("devloop.github_proxy_entity_view").commit_issue_subject_snapshot(core, plan.repo, plan.issue_number)
    ), 60)
  if commit_result.exit_code ~= 0 then
    error("github-devloop: git commit failed: " .. tostring(commit_result.stderr))
  end
  local branch_result = core.git_current_branch(worktree, 30)
  if branch_result.exit_code ~= 0 then
    error("github-devloop: git branch fact failed: " .. tostring(branch_result.stderr))
  end
  if tostring(branch_result.stdout or ""):gsub("%s+$", "") ~= plan.branch then
    error("github-devloop: PR origin fix branch mismatch")
  end
  local head_result = git("github-devloop").git_head_sha(worktree, 30)
  if head_result.exit_code ~= 0 then
    error("github-devloop: git head fact failed: " .. tostring(head_result.stderr))
  end
  local new_head_sha = tostring(head_result.stdout or ""):gsub("%s+$", "")
  if not require("devloop.pr_safety").is_safe_head_sha(new_head_sha) then
    error("github-devloop: unsafe fix head_sha")
  end
  if new_head_sha == plan.fix.reviewed_head_sha then
    return {
      kind = "review-meta",
      reason = "no-new-head",
      detail = result.stdout or result.stderr,
      outcome = "escalated: no-new-head",
      started_at = codex_started_at,
      finished_at = now(),
    }
  end
  return {
    kind = "reviewing",
    old_head_sha = plan.fix.reviewed_head_sha,
    new_head_sha = new_head_sha,
    reason = "fix pushed and PR head verified",
    summary = result.stdout or result.stderr,
    outcome = "completed: pushed for re-review",
    started_at = codex_started_at,
    finished_at = now(),
  }
end

local function validate_fix_write_gate_snapshot(repo, fix, branch, pr, reason_prefix, fail_closed)
  local rechecked_state = require("devloop.entity").current_entity_state(core, pr.comments, fix.proposal_id)
  if rechecked_state.state ~= "fixing" or tostring(rechecked_state.version or "") ~= tostring(fix.version) then
    core.log_cas_decision("fix", fix.proposal_id, rechecked_state, "fixing", "reviewing|review-meta", "skip-stale(write-gate)", tostring(reason_prefix) .. " issue state changed")
    return nil
  end
  if tostring(pr.state or ""):lower() ~= "open"
    or tostring(pr.head_ref_name or "") ~= branch
    or tostring(pr.head_sha or "") ~= tostring(fix.reviewed_head_sha)
    or not require("forge.merge.shared").is_same_repo_pr_head(pr, repo) then
    local outcome = fail_closed and "fail-closed(write-gate)" or "skip-stale(write-gate)"
    core.log_cas_decision("fix", fix.proposal_id, rechecked_state, "fixing", "reviewing|review-meta", outcome, tostring(reason_prefix) .. " PR fact changed or head repository missing")
    if fail_closed then
      error("github-devloop: write-time PR fact changed or head repository missing")
    end
    return nil
  end
  return pr
end

local function recheck_fix_write_gate(repo, fix, branch)
  local pr_recheck = core.gh_pr_view_fix(repo, fix.pr_number, 30)
  if pr_recheck.exit_code ~= 0 then
    error("github-devloop: gh pr fix recheck failed: " .. tostring(pr_recheck.stderr))
  end
  local rechecked_pr = parsers_pr.parse_pr_view_fix(core, pr_recheck.stdout)
  return validate_fix_write_gate_snapshot(repo, fix, branch, rechecked_pr, "write-time", true)
end

local function precheck_fix_write_gate(repo, fix, branch)
  local pr_precheck = core.gh_pr_view_fix_precheck(repo, fix.pr_number, 30)
  if pr_precheck.exit_code ~= 0 then
    error("github-devloop: gh pr fix precheck failed: " .. tostring(pr_precheck.stderr))
  end
  local prechecked_pr = parsers_pr.parse_pr_view_fix(core, pr_precheck.stdout)
  if validate_fix_write_gate_snapshot(repo, fix, branch, prechecked_pr, "pre-spawn", false) == nil then
    return false
  end
  return true
end

local function apply_fix_outcome(repo, issue_number, fix, branch, outcome)
  if outcome == nil then
    return
  end
  recheck_fix_write_gate(repo, fix, branch)
  if outcome.kind == "refix" then
    raise_stale_speculation_refix(
      repo,
      issue_number,
      fix,
      { state = "fixing", version = fix.version },
      outcome.current_predecessor_set or "none",
      outcome.reason or "speculative predecessor set changed"
    )
    return
  end
  if outcome.kind == "review-meta" then
    raise_review_meta(repo, issue_number, fix, outcome.reason, outcome.detail)
    return
  end
  if outcome.kind ~= "reviewing" then
    error("github-devloop: unknown fix outcome")
  end

  local push = core.git_push_branch(branch, 120)
  if push.exit_code ~= 0 then
    error("github-devloop: git push failed: " .. tostring(push.stderr))
  end
  local pushed_view = core.gh_pr_view_fix(repo, fix.pr_number, 30)
  if pushed_view.exit_code ~= 0 then
    error("github-devloop: gh pr pushed head view failed: " .. tostring(pushed_view.stderr))
  end
  local pushed_pr = parsers_pr.parse_pr_view_fix(core, pushed_view.stdout)
  if tostring(pushed_pr.state or ""):lower() ~= "open"
    or tostring(pushed_pr.head_ref_name or "") ~= branch
    or tostring(pushed_pr.head_sha or "") ~= outcome.new_head_sha
    or not require("forge.merge.shared").is_same_repo_pr_head(pushed_pr, repo) then
    error("github-devloop: pushed PR head verification failed")
  end

  raise_reviewing(repo, issue_number, fix, outcome.old_head_sha, outcome.new_head_sha, outcome.reason, outcome.summary)
end

local function act_fix(event)
  local fix = event.payload or {}
  if not v_fixing.is_supported_fixing(core, fix) then
    core.log_entry("fix", event, "unknown", core.payload_field(fix, "dedup_key"))
    core.log_cas_decision("fix", "unknown", { state = nil, version = nil }, "fixing", "reviewing|review-meta", "skip-foreign(payload)", "unsupported event payload")
    return
  end

  core.log_entry("fix", event, fix.proposal_id, fix.dedup_key)
  local entity = entity_lib.parse_entity_proposal_id(fix.proposal_id)
  if entity == nil then
    core.log_cas_decision("fix", fix.proposal_id, { state = nil, version = nil }, "fixing", "reviewing|review-meta", "skip-foreign(proposal_id)", "proposal_id is outside github-devloop")
    return
  end
  local repo = entity.repo
  local issue_number = entity.issue_number
  if entity.kind == "issue" and not m_claims.verify_pr_review_issue_claim(core, "fix", repo, issue_number, nil, fix.proposal_id) then
    return
  end

  local lock_key = entity_lib.transition_lock_key(fix.proposal_id)
  if lock_key == nil then
    core.log_cas_decision("fix", fix.proposal_id, { state = nil, version = nil }, "fixing", "reviewing|review-meta", "skip-foreign(proposal_id)", "no transition lock key")
    return
  end

  local attempt_plan = nil
  with_lock(lock_key, function()
    devloop_base.assert_trusted_bot_configured()
    local branches = config.branch_config(core)

    local pr_view = core.gh_pr_view_fix(repo, fix.pr_number, 30)
    if pr_view.exit_code ~= 0 then
      error("github-devloop: gh pr fix view failed: " .. tostring(pr_view.stderr))
    end
    local current_pr = parsers_pr.parse_pr_view_fix(core, pr_view.stdout)
    core.log_forged_markers("fix", fix.proposal_id, current_pr.comments)
    local reviewing_version = core.next_fix_version(fix.version)
    if core.has_state_marker(current_pr.comments, fix.proposal_id, "reviewing", reviewing_version) then
      core.log_cas_decision("fix", fix.proposal_id, { state = "reviewing", version = reviewing_version }, "fixing", "reviewing", "skip-idempotent(already at to_state)", "reviewing state marker for fix already visible")
      return
    end
    local state = require("devloop.entity").current_entity_state(core, current_pr.comments, fix.proposal_id)
    local transition = core.cyclic_transition_status(state, { "fixing" }, "reviewing", fix.version, reviewing_version)
    if transition == "pending" then
      core.log_cas_decision("fix", fix.proposal_id, state, "fixing", "reviewing", core.cas_outcome(state, transition, fix.version), "fixing state marker not yet visible")
      error("github-devloop: fixing state marker not yet visible for fix; retrying")
    end
    if transition == "idempotent" then
      core.log_cas_decision("fix", fix.proposal_id, state, "fixing", "reviewing", core.cas_outcome(state, transition, fix.version), "reviewing state marker for fix already visible")
      return
    end
    if state.state ~= "fixing" or transition == "stale" then
      core.log_cas_decision("fix", fix.proposal_id, state, "fixing", "reviewing", core.cas_outcome(state, transition, fix.version), "issue is not currently fixing")
      return
    end
    if tostring(state.version or "") ~= tostring(fix.version) then
      core.log_cas_decision("fix", fix.proposal_id, state, "fixing", "reviewing", "skip-stale(version-mismatch)", "fix event version does not match canonical issue marker")
      return
    end
    local reject_fact = m_facts.review_reject_fact(core, current_pr.comments, fix.proposal_id, fix.version)
    local meta_fix_fact = nil
    if reject_fact == nil then
      meta_fix_fact = m_facts.review_meta_fix_fact(core, current_pr.comments, fix.proposal_id, fix.version)
    end
    local merge_gate_fact = nil
    if reject_fact == nil and meta_fix_fact == nil then
      local merge_gate_candidate = m_facts.merge_gate_fix_fact(core, current_pr.comments, fix.proposal_id, fix.version)
      merge_gate_fact = m_facts.merge_gate_fix_fact(core, current_pr.comments, fix.proposal_id, fix.version, {
        review_proposal_id = fix.review_proposal_id,
        review_dedup_key = fix.review_dedup_key,
        gate_baseline_sha = fix.gate_baseline_sha,
        match_gate_baseline_sha = true,
      })
      if merge_gate_fact == nil then
        merge_gate_fact = merge_gate_candidate
      end
    end
    if reject_fact == nil and meta_fix_fact == nil and merge_gate_fact == nil then
      core.log_cas_decision("fix", fix.proposal_id, state, "fixing", "reviewing", "retry-pending(fix feedback marker not visible)", "reject review marker or review-meta fix marker missing")
      error("github-devloop: fix feedback marker not visible for fix; retrying")
    end
    local feedback_reason = nil
    if reject_fact ~= nil then
      if reject_fact.review_proposal_id ~= fix.review_proposal_id
        or reject_fact.review_dedup_key ~= fix.review_dedup_key
        or reject_fact.reviewed_head_sha ~= fix.reviewed_head_sha then
        core.log_cas_decision("fix", fix.proposal_id, state, "fixing", "reviewing", "skip-stale(review-fact-mismatch)", "fix event does not match canonical reject review marker")
        return
      end
      feedback_reason = reject_fact.review_reason
    elseif meta_fix_fact ~= nil then
      if meta_fix_fact.review_dedup_key ~= fix.review_dedup_key then
        core.log_cas_decision("fix", fix.proposal_id, state, "fixing", "reviewing", "skip-stale(review-meta-fact-mismatch)", "fix event does not match canonical review-meta fix marker")
        return
      end
      feedback_reason = meta_fix_fact.review_reason
    else
      if merge_gate_fact.review_proposal_id ~= fix.review_proposal_id
        or merge_gate_fact.review_dedup_key ~= fix.review_dedup_key
        or merge_gate_fact.reviewed_head_sha ~= fix.reviewed_head_sha then
        core.log_cas_decision("fix", fix.proposal_id, state, "fixing", "reviewing", "skip-stale(merge-gate-fact-mismatch)", "fix event does not match canonical merge-gate marker")
        return
      end
      if merge_gate_fact.gate_baseline_sha ~= fix.gate_baseline_sha then
        core.log_cas_decision("fix", fix.proposal_id, state, "fixing", "reviewing", "skip-stale(merge-gate-baseline-mismatch)", "fix event does not match canonical merge-gate baseline")
        return
      end
      feedback_reason = merge_gate_fact.review_reason
    end

    local origin = m_facts.pr_origin_fact(core, current_pr.comments)
    if origin == nil then
      origin = entity_lib.pr_native_origin(repo, fix.pr_number, current_pr)
    end
    if origin.proposal_id ~= fix.proposal_id
      or origin.repo ~= repo
      or tostring(origin.base_branch) ~= tostring(branches.integration)
      or tostring(current_pr.base_ref_name or "") ~= tostring(origin.base_branch)
      or tostring(current_pr.head_ref_name or "") ~= tostring(origin.branch) then
      core.log_cas_decision("fix", fix.proposal_id, state, "fixing", "reviewing", "skip-foreign(pr-origin)", "PR origin/link does not match immutable PR branch")
      return
    end
    local branch = origin.branch
    if tostring(current_pr.state or ""):lower() ~= "open" then
      core.log_cas_decision("fix", fix.proposal_id, state, "fixing", "reviewing", "skip-stale(pr-closed)", "re-derived PR is not open")
      return
    end
    if not require("forge.merge.shared").is_same_repo_pr_head(current_pr, repo) then
      core.log_cas_decision("fix", fix.proposal_id, state, "fixing", "reviewing", "fail-closed(head-repository)", "PR head repository is missing or not the target repository")
      error("github-devloop: PR head repository is missing or not the target repository")
    end
    if tostring(current_pr.head_sha or "") ~= tostring(fix.reviewed_head_sha) then
      local branch_head = core.git_branch_head(branch, 30)
      if branch_head.exit_code ~= 0 then
        core.log_cas_decision("fix", fix.proposal_id, state, "fixing", "reviewing", "retry-pending(head-advanced)", "PR head changed and deterministic branch head is not readable")
        error("github-devloop: PR head changed before fix marker and deterministic branch head is not readable")
      end
      local intended_head_sha = tostring(branch_head.stdout or ""):gsub("%s+$", "")
      if not require("devloop.pr_safety").is_safe_head_sha(intended_head_sha) then
        error("github-devloop: unsafe PR origin branch head sha")
      end
      if tostring(current_pr.head_sha or "") == intended_head_sha
        and tostring(current_pr.head_sha or "") ~= tostring(fix.reviewed_head_sha) then
        raise_reviewing(repo, issue_number, fix, fix.reviewed_head_sha, intended_head_sha, "push already visible; self-healing missing reviewing marker")
        return
      end
      core.log_cas_decision("fix", fix.proposal_id, state, "fixing", "reviewing", "skip-stale(head-advanced)", "PR head changed since rejected review")
      return
    end

    if not assert_fix_write_gate(fix, repo, issue_number) then
      return
    end

    local current_issue = {
      title = "PR #" .. tostring(fix.pr_number),
      body = "(PR-only fix context; issue backing is absent)",
      comments = current_pr.comments,
    }
    if issue_number ~= nil then
      local issue_view = core.gh_issue_view_fix(repo, issue_number, 30)
      if issue_view.exit_code ~= 0 then
        error("github-devloop: gh issue fix view failed: " .. tostring(issue_view.stderr))
      end
      current_issue = parsers_issue.parse_issue_view_fix(core, issue_view.stdout)
    end

    if merge_gate_fact ~= nil and tostring(merge_gate_fact.predecessor_set or "") ~= tostring(fix.predecessor_set or "") then
      core.log_cas_decision("fix", fix.proposal_id, state, "fixing", "reviewing", "skip-stale(predecessor-marker-mismatch)", "fix event does not match canonical merge-gate predecessor set")
      return
    end

    attempt_plan = {
      repo = repo,
      issue_number = issue_number,
      fix = fix,
      branches = branches,
      branch = branch,
      current_pr = current_pr,
      current_issue = current_issue,
      feedback_reason = feedback_reason,
      merge_gate_fact = merge_gate_fact,
      state = state,
      event_ts = event.ts,
      event_queue = event.queue,
    }
  end)
  if attempt_plan == nil then
    return
  end
  local pre_spawn_gate_ok = false
  with_lock(lock_key, function()
    pre_spawn_gate_ok = precheck_fix_write_gate(repo, fix, attempt_plan.branch)
    if pre_spawn_gate_ok and dispatch_live_run.dispatch_live_run_dedup(core, "fix", fix.proposal_id, fix.version) then
      core.log_cas_decision(
        "fix",
        fix.proposal_id,
        { state = "fixing", version = fix.version, stage_rank = core.stage_rank("fixing") },
        "fixing",
        "reviewing|review-meta",
        "skip-idempotent(live-exec-ref)",
        "matching fix codex run is still live"
      )
      pre_spawn_gate_ok = false
    end
  end)
  if not pre_spawn_gate_ok then
    return
  end
  local outcome = run_fix_attempt(attempt_plan)
  with_lock(lock_key, function()
    apply_fix_outcome(repo, issue_number, fix, attempt_plan.branch, outcome)
  end)
end

return saga.department(spec, {
  done = fix_done,
  act = act_fix,
  wrap = core.wrap_pipeline_failure,
  name = "fix",
})
