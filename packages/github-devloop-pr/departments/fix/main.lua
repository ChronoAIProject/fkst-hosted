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
local convergence_identity = require("contract.convergence_identity")
local workflow_codex = require("workflow.codex")
local dispatch_live_run = require("devloop.dispatch_live_run")
local conflict_telemetry = require("devloop.conflict_telemetry")
local context_bundle = require("devloop.context_bundle")
local config = require("devloop.config")
local merge_mechanics = require("departments.fix.merge_mechanics").make(core)
local ci_repair_attempts = require("core.ci_repair_attempts")
local ci_repair_retry = require("core.ci_repair_retry")
local ci_verdict = require("core.ci_verdict")
local fix_write_gate = require("departments.fix.write_gate")
local with_current_classification = ci_verdict.with_current_classification
local OWN_CI_RED = ci_verdict.OWN_CI_RED
local review_meta_caps = {
  build_comment = assert(rawget(core, "build_fix_review_meta_comment_request")),
  build_label = assert(rawget(core, "build_fix_review_meta_label_request")),
}

local dispatch_liveness = {
  restart_transition_table = function(...) return core.restart_transition_table(...) end,
  restart_row_receiver_liveness = function(...) return core.restart_row_receiver_liveness(...) end,
}

local payloads_builders = require("devloop.payloads.builders")
local conv_reconcile = require("devloop.convergence.reconcile")
local v_fixing = require("devloop.validators.fixing")
local m_facts = require("devloop.markers.facts")
local devloop_logging = require("devloop.logging")
local devloop_state = require("devloop.state")
local devloop_commands = require("devloop.commands")
local branch_worktree = merge_mechanics.branch_worktree
local merge_integration_for_fix = merge_mechanics.merge_integration_for_fix
local current_predecessors_for_fix = merge_mechanics.current_predecessors_for_fix
local merge_predecessor_entries_for_fix = merge_mechanics.merge_predecessor_entries_for_fix
local merge_speculative_predecessors_for_fix = merge_mechanics.merge_speculative_predecessors_for_fix
local assert_no_unmerged_paths = merge_mechanics.assert_no_unmerged_paths
local assert_no_conflict_markers = merge_mechanics.assert_no_conflict_markers
local bounded_fix_summary = merge_mechanics.bounded_fix_summary
local spec = {
  consumes = { "devloop_fixing" },
  produces = {
    "github-proxy.github_issue_label_request",
    "github-proxy.github_pr_comment_request",
    "devloop_review_meta",
    "devloop_fix_reconcile",
    "github-devloop-decompose.devloop_decompose",
  },
  stall_window = "10m",
  retry = { max_attempts = 12, base = "5s", cap = "30s" },
}

local function same_review_result_dedup(left, right)
  local left_canonical = devloop_base.canonical_pr_review_consensus_dedup_key(left)
  local right_canonical = devloop_base.canonical_pr_review_consensus_dedup_key(right)
  return left_canonical ~= nil and left_canonical == right_canonical
end

local git = git_adapter.production_handle

local function fix_done(_event)
  return false
end

local function raise_review_meta(...)
  return requests_review.raise_fix_review_meta(review_meta_caps, ...)
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

local function fix_at_next_attempt_version(fix)
  local review_meta = payloads_builders.build_devloop_review_meta_payload({
    proposal_id = fix.review_proposal_id,
    dedup_key = fix.review_dedup_key,
    source_ref = fix.source_ref,
  }, fix.proposal_id, devloop_state.next_fix_version(fix.version), fix.pr_number, 0, fix.source_ref)
  local advanced = {}
  for key, value in pairs(fix) do
    advanced[key] = value
  end
  advanced.version = review_meta.version
  advanced.dedup_key = review_meta.dedup_key
  return advanced
end

local function assert_fix_write_gate(fix, repo, issue_number)
  local write_enabled = config.write_mode() == "real"
  if write_enabled then
    return true
  end
  devloop_logging.log_line("info", "fix", fix.proposal_id, "OUTBOUND", {
    "mode=dry-run",
    "repo=" .. tostring(repo),
    "issue=" .. tostring(issue_number),
    "pr=" .. tostring(fix.pr_number),
    "reason=PR fix requires FKST_GITHUB_WRITE=1 before codex",
  })
  return false
end

local function branch_head_if_ahead(base_head_sha, branch)
  local ahead_result = devloop_commands.git_branch_ahead_count(base_head_sha, branch, 30)
  if ahead_result.exit_code ~= 0 then
    error("github-devloop: git-branch-ahead-check-failed: git branch ahead check failed: " .. tostring(ahead_result.stderr))
  end
  local ahead_count = tonumber(tostring(ahead_result.stdout or ""):match("%d+"))
  if ahead_count == nil or ahead_count <= 0 then
    return nil
  end
  local head_result = devloop_commands.git_branch_head(branch, 30)
  if head_result.exit_code ~= 0 then
    error("github-devloop: git-branch-head-check-failed: git branch head check failed: " .. tostring(head_result.stderr))
  end
  local branch_head_sha = tostring(head_result.stdout or ""):gsub("%s+$", "")
  if not require("devloop.pr_safety").is_safe_head_sha(branch_head_sha) then
    error("github-devloop: deterministic-branch-head-unsafe: unsafe deterministic branch head sha")
  end
  if branch_head_sha == base_head_sha then
    return nil
  end
  return branch_head_sha
end

local function validate_fix_write_gate_snapshot(repo, fix, branch, pr, reason_prefix, fail_closed)
  local state = require("devloop.entity").current_entity_state(pr.comments, fix.proposal_id)
  if state.state ~= "fixing" or tostring(state.version or "") ~= tostring(fix.version) then
    devloop_logging.log_cas_decision(
      "fix",
      fix.proposal_id,
      state,
      "fixing",
      "reviewing|review-meta",
      "skip-stale(write-gate)",
      tostring(reason_prefix) .. " issue state changed"
    )
    return nil
  end
  return fix_write_gate.validate(repo, fix, branch, pr, state, reason_prefix, fail_closed)
end

local function run_fix_attempt(plan)
  local worktree = branch_worktree(plan.repo, plan.issue_number, plan.fix.version, plan.branch)
  local merge_context, speculative_reason, speculative_current_set
  if plan.speculative_predecessors ~= nil then
    merge_context, speculative_reason = merge_predecessor_entries_for_fix(
      worktree,
      plan.branches.integration,
      plan.speculative_predecessors,
      plan.speculative_current_set
    )
    speculative_current_set = plan.speculative_current_set
  else
    merge_context, speculative_reason, speculative_current_set = merge_speculative_predecessors_for_fix(
      worktree,
      plan.repo,
      plan.branches.integration,
      plan.fix,
      plan.current_pr
    )
  end
  if merge_context == nil and speculative_reason ~= "not-speculative" then
    if speculative_reason == "predecessor-set-mismatch" then
      return {
        kind = "refix",
        current_predecessor_set = speculative_current_set or "none",
        reason = "speculative predecessor set changed",
      }
    end
    devloop_logging.log_cas_decision("fix", plan.fix.proposal_id, plan.state, "fixing", "reviewing", "skip-stale(" .. tostring(speculative_reason) .. ")", "speculative predecessor set is no longer current")
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
    conflict_telemetry.log_conflict_files("fix", plan.fix.proposal_id, plan.fix.pr_number, merge_context.unmerged_paths)
  end
  local dispatch = function()
  local codex_started_at = now()
  devloop_logging.log_codex_start("fix", plan.fix.proposal_id, "fix")
  local content_fetch = context_bundle.context_fetch_from_bundle(core, {
    dept = "fix",
    repo = plan.repo,
    issue_number = plan.issue_number,
    pr_number = plan.fix.pr_number,
    proposal_id = plan.fix.proposal_id,
    version = plan.fix.dedup_key,
    tick = plan.event_ts,
  })
  local result = workflow_codex.dispatch(convergence_identity.from_parts("fix", plan.fix.proposal_id, plan.fix.work_unit_key, {
    angle_lane = "worker",
  }), {
    prompt = core.build_fix_prompt(plan.fix, plan.current_issue, plan.feedback_reason, plan.fix.framing, content_fetch, merge_context),
    worktree = worktree,
    timeout = 2 * 60 * 60,  -- 2h: fix loops code+test (#1481)
    sync = true,
  })
  if type(result) == "table" and result.deferred then
    devloop_logging.log_codex_result("fix", plan.fix.proposal_id, "fix", result, "result=deferred", nil)
    return nil
  end
  if type(result) ~= "table" or result.exit_code ~= 0 then
    local stderr = type(result) == "table" and result.stderr or "nil result"
    devloop_logging.log_codex_result("fix", plan.fix.proposal_id, "fix", result, nil, stderr, {
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
  devloop_logging.log_codex_result("fix", plan.fix.proposal_id, "fix", result, "result=completed", nil)
  assert_no_unmerged_paths(worktree)
  assert_no_conflict_markers(worktree)

  local status = devloop_commands.git_status(worktree, 30)
  if status.exit_code ~= 0 then
    error("github-devloop: git-status-failed: git status failed: " .. tostring(status.stderr))
  end
  if tostring(status.stdout or "") == "" then
    local existing_head_sha = branch_head_if_ahead(plan.fix.reviewed_head_sha, plan.branch)
    if existing_head_sha ~= nil then
      devloop_logging.log_codex_result("fix", plan.fix.proposal_id, "fix", result, "result=reusing-existing-head", nil)
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
    devloop_logging.log_codex_result("fix", plan.fix.proposal_id, "fix", result, nil, "no-changes", {
      queue = plan.event_queue,
      source_ref = plan.fix.source_ref,
      terminal = false,
    })
    return {
      kind = "review-meta",
      completed_without_new_head = true,
      reason = "no-fix",
      detail = result.stdout or result.stderr,
      outcome = "escalated: no-fix",
      started_at = codex_started_at,
      finished_at = now(),
    }
  end

  local add_result = devloop_commands.git_add_all(worktree, 30)
  if add_result.exit_code ~= 0 then
    error("github-devloop: git-add-failed: git add failed: " .. tostring(add_result.stderr))
  end
  local commit_result = devloop_commands.git_commit(worktree, payloads_builders.fix_commit_subject(
      plan.issue_number,
      require("devloop.github_proxy_entity_view").commit_issue_subject_snapshot(plan.repo, plan.issue_number)
    ), 60)
  if commit_result.exit_code ~= 0 then
    error("github-devloop: git-commit-failed: git commit failed: " .. tostring(commit_result.stderr))
  end
  local branch_result = devloop_commands.git_current_branch(worktree, 30)
  if branch_result.exit_code ~= 0 then
    error("github-devloop: git-branch-fact-failed: git branch fact failed: " .. tostring(branch_result.stderr))
  end
  if tostring(branch_result.stdout or ""):gsub("%s+$", "") ~= plan.branch then
    error("github-devloop: fix-branch-mismatch: PR origin fix branch mismatch")
  end
  local head_result = git("github-devloop").git_head_sha(worktree, 30)
  if head_result.exit_code ~= 0 then
    error("github-devloop: git-head-fact-failed: git head fact failed: " .. tostring(head_result.stderr))
  end
  local new_head_sha = tostring(head_result.stdout or ""):gsub("%s+$", "")
  if not require("devloop.pr_safety").is_safe_head_sha(new_head_sha) then
    error("github-devloop: fix-head-unsafe: unsafe fix head_sha")
  end
  if new_head_sha == plan.fix.reviewed_head_sha then
    return {
      kind = "review-meta",
      completed_without_new_head = true,
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
  if plan.fix.repair_input ~= "ci-failure" then
    return dispatch()
  end
  local outcome, mismatch, observed_pr = with_current_classification(
    plan.repo,
    plan.fix.pr_number,
    plan.fix.reviewed_head_sha,
    function(classification)
      local current_pr = classification.current_pr
      local authorized = validate_fix_write_gate_snapshot(
        plan.repo, plan.fix, plan.branch, current_pr, "pre-dispatch", false
      )
      if authorized == nil then
        return nil
      end
      if classification.kind ~= OWN_CI_RED then
        return {
          kind = "reviewing-current",
          current_pr = current_pr,
          reason = "own-CI gate no longer requires repair: " .. tostring(classification.reason),
        }
      end
      plan.current_pr = current_pr
      return dispatch()
    end,
    {
      dept = "fix",
      proposal_id = plan.fix.proposal_id,
      error_class = "gh-pr-fix-dispatch-view-failed",
    }
  )
  if mismatch == "head-mismatch" then
    validate_fix_write_gate_snapshot(plan.repo, plan.fix, plan.branch, observed_pr, "pre-dispatch", false)
    return nil
  end
  return outcome
end

local function recheck_fix_write_gate(repo, fix, branch)
  local pr_recheck = devloop_commands.gh_pr_view_fix(repo, fix.pr_number, 30)
  if pr_recheck.exit_code ~= 0 then
    error("github-devloop: gh-pr-fix-recheck-failed: gh pr fix recheck failed: " .. tostring(pr_recheck.stderr))
  end
  local rechecked_pr = parsers_pr.parse_pr_view_fix(pr_recheck.stdout)
  return validate_fix_write_gate_snapshot(repo, fix, branch, rechecked_pr, "write-time", true)
end

local function precheck_fix_write_gate(repo, fix, branch)
  local pr_precheck = devloop_commands.gh_pr_view_fix_precheck(repo, fix.pr_number, 30)
  if pr_precheck.exit_code ~= 0 then
    error("github-devloop: gh-pr-fix-precheck-failed: gh pr fix precheck failed: " .. tostring(pr_precheck.stderr))
  end
  local prechecked_pr = parsers_pr.parse_pr_view_fix(pr_precheck.stdout)
  local prechecked, prechecked_state = validate_fix_write_gate_snapshot(repo, fix, branch, prechecked_pr, "pre-spawn", false)
  if prechecked == nil then
    return nil
  end
  return prechecked, prechecked_state
end

local function apply_fix_outcome(repo, issue_number, fix, branch, outcome)
  if outcome == nil then
    return
  end
  local rechecked_pr, current_state = recheck_fix_write_gate(repo, fix, branch)
  if current_state == nil then
    return
  end
  if outcome.kind == "reviewing-current" then
    raise_reviewing(
      repo,
      issue_number,
      fix,
      fix.reviewed_head_sha,
      rechecked_pr.head_sha,
      outcome.reason
    )
    return
  end
  if outcome.kind == "refix" then
    ci_repair_retry.raise_speculative(core,
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
    if fix.repair_input == "ci-failure" then
      ci_repair_attempts.raise_attempt_record(repo, fix, outcome.reason or "no-repair", outcome.detail)
      return
    end
    if outcome.completed_without_new_head == true then
      local fix_round = devloop_state.version_fix_round(current_state.version)
      if fix_round >= config.max_fix_rounds() then
        local fix_reconcile = conv_reconcile.build_devloop_fix_reconcile_payload({
          proposal_id = fix.proposal_id,
          review_proposal_id = fix.review_proposal_id,
          review_dedup_key = fix.review_dedup_key,
          reviewed_head_sha = fix.reviewed_head_sha,
          pr_number = fix.pr_number,
          source_ref = fix.source_ref,
        }, current_state.version)
        local decompose = payloads_builders.build_devloop_decompose_payload(fix_reconcile)
        devloop_logging.log_cas_decision("fix", fix.proposal_id, current_state, "fixing", "blocked", "applied(fix-loop-max-rounds)", "completed fix attempt produced no new head: " .. tostring(outcome.reason))
        devloop_logging.log_raise("fix", fix.proposal_id, "devloop_fix_reconcile", fix_reconcile)
        devloop_logging.log_raise("fix", fix.proposal_id, "github-devloop-decompose.devloop_decompose", decompose)
        return
      end
      raise_review_meta(repo, issue_number, fix_at_next_attempt_version(fix), outcome.reason, outcome.detail)
      return
    end
    raise_review_meta(repo, issue_number, fix, outcome.reason, outcome.detail)
    return
  end
  if outcome.kind ~= "reviewing" then
    error("github-devloop: fix-outcome-unknown: unknown fix outcome")
  end

  local push = devloop_commands.git_push_branch(branch, 120)
  if push.exit_code ~= 0 then
    error("github-devloop: git-push-failed: git push failed: " .. tostring(push.stderr))
  end
  local pushed_view = devloop_commands.gh_pr_view_fix(repo, fix.pr_number, 30)
  if pushed_view.exit_code ~= 0 then
    error("github-devloop: gh-pr-pushed-head-view-failed: gh pr pushed head view failed: " .. tostring(pushed_view.stderr))
  end
  local pushed_pr = parsers_pr.parse_pr_view_fix(pushed_view.stdout)
  if tostring(pushed_pr.state or ""):lower() ~= "open"
    or tostring(pushed_pr.head_ref_name or "") ~= branch
    or tostring(pushed_pr.head_sha or "") ~= outcome.new_head_sha
    or not require("forge.merge.shared").is_same_repo_pr_head(pushed_pr, repo) then
    error("github-devloop: pushed-pr-head-mismatch: pushed PR head verification failed")
  end

  raise_reviewing(repo, issue_number, fix, outcome.old_head_sha, outcome.new_head_sha, outcome.reason, outcome.summary)
end

local function act_fix(event)
  local fix = event.payload or {}
  if not v_fixing.is_supported_fixing(fix) then
    devloop_logging.log_entry("fix", event, "unknown", devloop_logging.payload_field(fix, "dedup_key"))
    devloop_logging.log_cas_decision("fix", "unknown", { state = nil, version = nil }, "fixing", "reviewing|review-meta", "skip-foreign(payload)", "unsupported event payload")
    return
  end

  devloop_logging.log_entry("fix", event, fix.proposal_id, fix.dedup_key)
  local entity = entity_lib.parse_entity_proposal_id(fix.proposal_id)
  if entity == nil then
    devloop_logging.log_cas_decision("fix", fix.proposal_id, { state = nil, version = nil }, "fixing", "reviewing|review-meta", "skip-foreign(proposal_id)", "proposal_id is outside github-devloop")
    return
  end
  local repo = entity.repo
  local issue_number = entity.issue_number
  if entity.kind == "issue" and not m_claims.verify_pr_review_issue_claim("fix", repo, issue_number, nil, fix.proposal_id) then
    return
  end

  local lock_key = entity_lib.transition_lock_key(fix.proposal_id)
  if lock_key == nil then
    devloop_logging.log_cas_decision("fix", fix.proposal_id, { state = nil, version = nil }, "fixing", "reviewing|review-meta", "skip-foreign(proposal_id)", "no transition lock key")
    return
  end

  local attempt_plan = nil
  with_lock(lock_key, function()
    devloop_base.assert_trusted_bot_configured()
    local branches = config.branch_config()

    local pr_view = devloop_commands.gh_pr_view_fix(repo, fix.pr_number, 30)
    if pr_view.exit_code ~= 0 then
      error("github-devloop: gh-pr-fix-view-failed: gh pr fix view failed: " .. tostring(pr_view.stderr))
    end
    local current_pr = parsers_pr.parse_pr_view_fix(pr_view.stdout)
    devloop_logging.log_forged_markers("fix", fix.proposal_id, current_pr.comments)
    local reviewing_version = devloop_state.next_fix_version(fix.version)
    if devloop_state.has_state_marker(current_pr.comments, fix.proposal_id, "reviewing", reviewing_version) then
      devloop_logging.log_cas_decision("fix", fix.proposal_id, { state = "reviewing", version = reviewing_version }, "fixing", "reviewing", "skip-idempotent(already at to_state)", "reviewing state marker for fix already visible")
      return
    end
    local state = require("devloop.entity").current_entity_state(current_pr.comments, fix.proposal_id)
    local transition = devloop_state.cyclic_transition_status(state, { "fixing" }, "reviewing", fix.version, reviewing_version)
    if transition == "pending" then
      devloop_logging.log_cas_decision("fix", fix.proposal_id, state, "fixing", "reviewing", devloop_state.cas_outcome(state, transition, fix.version), "fixing state marker not yet visible")
      error("github-devloop: fixing-marker-missing: fixing state marker not yet visible for fix; retrying")
    end
    if transition == "idempotent" then
      devloop_logging.log_cas_decision("fix", fix.proposal_id, state, "fixing", "reviewing", devloop_state.cas_outcome(state, transition, fix.version), "reviewing state marker for fix already visible")
      return
    end
    if state.state ~= "fixing" or transition == "stale" then
      devloop_logging.log_cas_decision("fix", fix.proposal_id, state, "fixing", "reviewing", devloop_state.cas_outcome(state, transition, fix.version), "issue is not currently fixing")
      return
    end
    if tostring(state.version or "") ~= tostring(fix.version) then
      devloop_logging.log_cas_decision("fix", fix.proposal_id, state, "fixing", "reviewing", "skip-stale(version-mismatch)", "fix event version does not match canonical issue marker")
      return
    end
    local reject_fact = m_facts.review_reject_fact(current_pr.comments, fix.proposal_id, fix.version)
    local meta_fix_fact = nil
    if reject_fact == nil then
      meta_fix_fact = m_facts.review_meta_fix_fact(current_pr.comments, fix.proposal_id, fix.version)
    end
    local merge_gate_fact = nil
    if reject_fact == nil and meta_fix_fact == nil then
      local merge_gate_candidate = m_facts.merge_gate_fix_fact(current_pr.comments, fix.proposal_id, fix.version)
      local event_fact_visible
      merge_gate_fact, event_fact_visible = m_facts.merge_gate_fix_fact(current_pr.comments, fix.proposal_id, fix.version, {
        review_proposal_id = fix.review_proposal_id,
        review_dedup_key = fix.review_dedup_key,
        reviewed_head_sha = fix.reviewed_head_sha,
        gate_baseline_sha = fix.gate_baseline_sha,
        match_gate_baseline_sha = true,
        predecessor_set = fix.predecessor_set,
        match_predecessor_set = true,
        ci_failure_key = fix.ci_failure_key,
        match_ci_failure_key = true,
      })
      if merge_gate_fact == nil and merge_gate_candidate ~= nil then
        if event_fact_visible then
          devloop_logging.log_cas_decision("fix", fix.proposal_id, state, "fixing", "reviewing", "skip-stale(superseded-merge-gate-fact)", "a newer canonical merge-gate fact supersedes this fix event")
          return
        end
        devloop_logging.log_cas_decision("fix", fix.proposal_id, state, "fixing", "reviewing", "fail-closed(merge-gate-fact-mismatch)", "active fix event does not match any trusted merge-gate fact")
        error("github-devloop: active-merge-gate-fact-mismatch: active fix event does not match any trusted merge-gate fact")
      end
    end
    if reject_fact == nil and meta_fix_fact == nil and merge_gate_fact == nil then
      devloop_logging.log_cas_decision("fix", fix.proposal_id, state, "fixing", "reviewing", "retry-pending(fix feedback marker not visible)", "reject review marker or review-meta fix marker missing")
      error("github-devloop: fix-feedback-marker-missing: fix feedback marker not visible for fix; retrying")
    end
    local feedback_reason = nil
    if reject_fact ~= nil then
      if reject_fact.review_proposal_id ~= fix.review_proposal_id
        or not same_review_result_dedup(reject_fact.review_dedup_key, fix.review_dedup_key)
        or reject_fact.reviewed_head_sha ~= fix.reviewed_head_sha then
        devloop_logging.log_cas_decision("fix", fix.proposal_id, state, "fixing", "reviewing", "fail-closed(review-fact-mismatch)", "active fix event does not match canonical reject review marker")
        error("github-devloop: active-review-fact-mismatch: active fix event does not match canonical reject review marker")
      end
      feedback_reason = reject_fact.review_reason
    elseif meta_fix_fact ~= nil then
      if meta_fix_fact.review_dedup_key ~= fix.review_dedup_key then
        devloop_logging.log_cas_decision("fix", fix.proposal_id, state, "fixing", "reviewing", "fail-closed(review-meta-fact-mismatch)", "active fix event does not match canonical review-meta fix marker")
        error("github-devloop: active-review-meta-fact-mismatch: active fix event does not match canonical review-meta fix marker")
      end
      feedback_reason = meta_fix_fact.review_reason
    else
      feedback_reason = merge_gate_fact.review_reason
    end

    local origin = m_facts.pr_origin_fact(current_pr.comments)
    if origin == nil then
      origin = entity_lib.pr_native_origin(repo, fix.pr_number, current_pr)
    end
    if origin.proposal_id ~= fix.proposal_id
      or origin.repo ~= repo
      or tostring(origin.base_branch) ~= tostring(branches.integration)
      or tostring(current_pr.base_ref_name or "") ~= tostring(origin.base_branch)
      or tostring(current_pr.head_ref_name or "") ~= tostring(origin.branch) then
      devloop_logging.log_cas_decision("fix", fix.proposal_id, state, "fixing", "reviewing", "skip-foreign(pr-origin)", "PR origin/link does not match immutable PR branch")
      return
    end
    local branch = origin.branch
    if tostring(current_pr.state or ""):lower() ~= "open" then
      devloop_logging.log_cas_decision("fix", fix.proposal_id, state, "fixing", "reviewing", "skip-stale(pr-closed)", "re-derived PR is not open")
      return
    end
    if not require("forge.merge.shared").is_same_repo_pr_head(current_pr, repo) then
      devloop_logging.log_cas_decision("fix", fix.proposal_id, state, "fixing", "reviewing", "fail-closed(head-repository)", "PR head repository is missing or not the target repository")
      error("github-devloop: pr-head-repository-invalid: PR head repository is missing or not the target repository")
    end
    if tostring(current_pr.head_sha or "") ~= tostring(fix.reviewed_head_sha) then
      local branch_head = devloop_commands.git_branch_head(branch, 30)
      if branch_head.exit_code ~= 0 then
        devloop_logging.log_cas_decision("fix", fix.proposal_id, state, "fixing", "reviewing", "retry-pending(head-advanced)", "PR head changed and deterministic branch head is not readable")
        error("github-devloop: pr-head-advanced: PR head changed before fix marker and deterministic branch head is not readable")
      end
      local intended_head_sha = tostring(branch_head.stdout or ""):gsub("%s+$", "")
      if not require("devloop.pr_safety").is_safe_head_sha(intended_head_sha) then
        error("github-devloop: pr-origin-branch-head-unsafe: unsafe PR origin branch head sha")
      end
      if tostring(current_pr.head_sha or "") == intended_head_sha
        and tostring(current_pr.head_sha or "") ~= tostring(fix.reviewed_head_sha) then
        raise_reviewing(repo, issue_number, fix, fix.reviewed_head_sha, intended_head_sha, "push already visible; self-healing missing reviewing marker")
        return
      end
      devloop_logging.log_cas_decision("fix", fix.proposal_id, state, "fixing", "reviewing", "skip-stale(head-advanced)", "PR head changed since rejected review")
      return
    end

    if fix.repair_input == "ci-failure" and ci_repair_attempts.fact(current_pr.comments, fix) ~= nil then
      devloop_logging.log_cas_decision("fix", fix.proposal_id, state, "fixing", "fixing", "skip-idempotent(ci-repair-attempt-visible)", "completed CI repair round fact is visible; replay admission owns continuation")
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
      local issue_view = devloop_commands.gh_issue_view_fix(repo, issue_number, 30)
      if issue_view.exit_code ~= 0 then
        error("github-devloop: gh-issue-fix-view-failed: gh issue fix view failed: " .. tostring(issue_view.stderr))
      end
      current_issue = parsers_issue.parse_issue_view_fix(core, issue_view.stdout)
    end

    local speculative_predecessors = nil
    local speculative_current_set = nil
    if fix.predecessor_set ~= nil then
      speculative_predecessors, speculative_current_set = current_predecessors_for_fix(repo, branches.integration, fix, current_pr)
      if speculative_predecessors ~= nil and tostring(speculative_current_set) ~= tostring(fix.predecessor_set) then
        ci_repair_retry.raise_speculative(core,
          repo,
          issue_number,
          fix,
          state,
          speculative_current_set,
          "speculative predecessor set changed"
        )
        return
      end
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
      speculative_predecessors = speculative_predecessors,
      speculative_current_set = speculative_current_set,
    }
  end)
  if attempt_plan == nil then
    return
  end
  local pre_spawn_gate_ok = false
  with_lock(lock_key, function()
    local prechecked_pr, prechecked_state = precheck_fix_write_gate(repo, fix, attempt_plan.branch)
    pre_spawn_gate_ok = prechecked_pr ~= nil
    if pre_spawn_gate_ok and dispatch_live_run.dispatch_live_run_dedup(dispatch_liveness, "fix", fix.proposal_id, fix.work_unit_key, {
      state = prechecked_state,
      current_pr = prechecked_pr,
      proposal_id = fix.proposal_id,
      work_unit_key = fix.work_unit_key,
      now_seconds = now(),
    }) then
      devloop_logging.log_cas_decision(
        "fix",
        fix.proposal_id,
        { state = "fixing", version = fix.version, stage_rank = devloop_state.stage_rank("fixing") },
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
  if outcome == nil then
    return
  end
  with_lock(lock_key, function()
    apply_fix_outcome(repo, issue_number, fix, attempt_plan.branch, outcome)
  end)
end

return saga.department(spec, {
  done = fix_done,
  act = act_fix,
  wrap = devloop_logging.wrap_pipeline_failure,
  name = "fix",
})
