local core = require("core")

local M = {}

M.spec = {
  consumes = { "github-proxy.github_entity_changed", "github-proxy.github_pr_opened" },
  produces = {
    "github-proxy.github_issue_label_request",
    "github-proxy.github_pr_comment_request",
    "devloop_reviewing",
    "devloop_fixing",
    "devloop_decompose",
    "devloop_merge_ready",
    "devloop_reconcile",
    "devloop_review_reconcile",
    "devloop_timeout_reconcile",
  },
  stall_window = "30s",
  retry = { max_attempts = 12, base = "5s", cap = "30s" },
}

local function pr_source_ref(repo, pr_number)
  return core.pr_source_ref(repo, pr_number)
end

local function pr_context(event)
  local payload = event.payload or {}
  if core.is_supported_pr(payload) then
    return {
      source = "poll",
      repo = payload.repo,
      number = payload.number,
      dedup_key = payload.dedup_key,
      source_ref = payload.source_ref,
    }
  end
  if core.is_supported_pr_opened(payload) then
    return {
      source = "direct-opened",
      repo = payload.repo,
      number = payload.pr_number,
      dedup_key = payload.dedup_key,
      source_ref = payload.source_ref,
      proposal_id = payload.proposal_id,
      issue_number = payload.issue_number,
      impl_version = payload.impl_version,
      branch = payload.branch,
      head_sha = payload.head_sha,
      base_branch = payload.base_branch,
    }
  end
  return nil
end

local function origin_from_pr(repo, pr_number, current_pr)
  local origin = core.pr_origin_fact(current_pr.comments)
  if origin ~= nil then
    return origin, true
  end
  return core.pr_native_origin(repo, pr_number, current_pr), false
end

local function origin_matches_pr(origin, current_pr, repo, branches, require_issue_backing)
  if origin.repo ~= repo then
    return false, "repo"
  end
  if require_issue_backing and origin.issue_number == nil then
    return false, "issue"
  end
  if tostring(current_pr.head_ref_name or "") ~= tostring(origin.branch) then
    return false, "head"
  end
  if tostring(current_pr.base_ref_name or "") ~= tostring(origin.base_branch) then
    return false, "base"
  end
  if origin.base_branch ~= nil
    and tostring(origin.base_branch or "") ~= tostring(branches.integration) then
    return false, "base"
  end
  return true, "ok"
end

local function maybe_issue_label_hint(origin, state, source_ref)
  if origin.issue_number == nil then
    return
  end
  local label_request = core.build_reconcile_state_label_request(origin.repo, origin.issue_number, origin.proposal_id, state.state, state.version, source_ref)
  local add_labels, remove_labels = core.state_label_changes(state.state)
  core.log_apply("observe_pr", origin.proposal_id, state.state, state.version, { add = add_labels, remove = remove_labels }, {
    "github-proxy.github_issue_label_request",
  })
  core.log_raise("observe_pr", origin.proposal_id, "github-proxy.github_issue_label_request", label_request)
end

local function maybe_pr_label_hint(origin, pr_number, current_pr, state, source_ref)
  if state.state == nil or core.state_label_hint_matches(current_pr.labels, state.state) then
    return
  end
  local label_request = core.build_reconcile_pr_state_label_request(origin.repo, origin.issue_number, pr_number, origin.proposal_id, state.state, state.version, source_ref)
  local add_labels, remove_labels = core.state_label_changes(state.state)
  core.log_apply("observe_pr", origin.proposal_id, state.state, state.version, { add = add_labels, remove = remove_labels }, {
    "github-proxy.github_issue_label_request",
  })
  core.log_raise("observe_pr", origin.proposal_id, "github-proxy.github_issue_label_request", label_request)
end

local function maybe_label_hints(origin, pr_number, current_pr, state, pr_source_ref_value)
  local issue_source_ref_value = origin.issue_number ~= nil and core.issue_source_ref(origin.repo, origin.issue_number) or nil
  maybe_issue_label_hint(origin, state, issue_source_ref_value)
  maybe_pr_label_hint(origin, pr_number, current_pr, state, pr_source_ref_value)
end

local function issue_comments_for_origin(origin)
  if origin.issue_number == nil then
    return nil
  end
  local issue_view = core.gh_exec({ cmd = core.gh_issue_view_result_cmd(origin.repo, origin.issue_number), timeout = 30 })
  if issue_view.exit_code ~= 0 then
    error("github-devloop: gh issue result view failed: " .. tostring(issue_view.stderr))
  end
  return core.parse_issue_view_result(issue_view.stdout).comments
end

local function issue_reviewing_for_origin(origin)
  if origin.issue_number == nil then
    return nil
  end
  local issue_view = core.gh_exec({ cmd = core.gh_issue_view_reviewing_cmd(origin.repo, origin.issue_number), timeout = 30 })
  if issue_view.exit_code ~= 0 then
    error("github-devloop: gh issue reviewing view failed: " .. tostring(issue_view.stderr))
  end
  return core.parse_issue_view_reviewing(issue_view.stdout)
end

local function issue_claim_for_origin(origin)
  if origin.issue_number == nil then
    return nil
  end
  return { assignees = core.read_current_issue_assignees(origin.repo, origin.issue_number) }
end

local function raise_current_state(origin, pr_number, current_pr, state, source_ref, known_issue)
  if state.state == "fixing" and tostring(current_pr.state or ""):lower() ~= "open" then
    core.log_cas_decision("observe_pr", origin.proposal_id, state, "fixing", "fixing", "skip-stale(pr-closed)", "re-derived PR is not open")
    return false
  end
  local issue_comments = known_issue and known_issue.comments or nil
  if issue_comments == nil and state.state == "fixing" then
    issue_comments = issue_comments_for_origin(origin)
  end
  if state.state == "blocked" and core.decomposed_fact(current_pr.comments, origin.proposal_id, state.version, pr_number) == nil then
    core.log_cas_decision("observe_pr", origin.proposal_id, state, "blocked", "decomposed", "skip-foreign(decomposed)", "decomposed marker is not visible")
    return false
  end
  local issue_source_ref = origin.issue_number ~= nil and core.issue_source_ref(origin.repo, origin.issue_number) or source_ref
  return core.replay_from_table("observe_pr", {
    repo = origin.repo,
    number = origin.issue_number,
    source_ref = issue_source_ref,
    _replay_issue_comments = issue_comments,
  }, state, core.restart_transition_row(state.state), {
    proposal_id = origin.proposal_id,
    current = { comments = issue_comments or {} },
    current_pr = current_pr,
    link = {
      proposal_id = origin.proposal_id,
      pr_number = pr_number,
      branch = origin.branch,
      impl_version = origin.impl_version,
      base_branch = origin.base_branch,
    },
    source_ref = source_ref,
  })
end

local function is_stalled_reviewing(current_pr, origin, pr_number, state)
  if state.state ~= "reviewing" or not core._is_git_sha(current_pr.head_sha) then
    return false
  end
  local review_proposal_id = core.pr_review_proposal_id(origin.repo, pr_number, state.version, current_pr.head_sha)
  local review_version = core.safe_version_segment(state.version)
  local sr_digest = core.source_ref_digest(core.pr_source_ref(origin.repo, pr_number))
  local facts = core.review_converge_round_facts(
    current_pr.comments,
    review_proposal_id,
    origin.proposal_id,
    review_version,
    current_pr.head_sha,
    sr_digest
  )
  local round = core.max_converge_round(facts)
  return core.is_true_stall(facts, round)
end

local function maybe_apply_rereview_command(origin, pr_number, current_pr, state, source_ref)
  local command = core.operator_command_fact(current_pr.comments, "rereview")
  if command == nil then
    return false
  end
  if core.has_operator_command_response(current_pr.comments, command) then
    core.log_cas_decision("observe_pr", origin.proposal_id, state, "blocked|review-meta|reviewing", "reviewing", "skip-idempotent(command-response-visible)", "operator command response marker is already visible")
    return false
  end
  if state.state ~= "blocked" and state.state ~= "review-meta" and state.state ~= "reviewing" then
    core.log_cas_decision("observe_pr", origin.proposal_id, state, "blocked|review-meta|reviewing", "reviewing", "refused(invalid-state)", "operator rereview precondition failed")
    local refusal = core.build_operator_command_refusal_request(
      origin.repo,
      pr_number,
      command,
      "rereview requires blocked, review-meta, or stalled reviewing state",
      source_ref
    )
    core.log_raise("observe_pr", origin.proposal_id, "github-proxy.github_pr_comment_request", refusal)
    return true
  end
  if state.state == "reviewing" and not is_stalled_reviewing(current_pr, origin, pr_number, state) then
    core.log_cas_decision("observe_pr", origin.proposal_id, state, "blocked|review-meta|stalled-reviewing", "reviewing", "refused(active-reviewing)", "operator rereview requires stalled reviewing")
    local refusal = core.build_operator_command_refusal_request(
      origin.repo,
      pr_number,
      command,
      "rereview requires blocked, review-meta, or stalled reviewing state",
      source_ref
    )
    core.log_raise("observe_pr", origin.proposal_id, "github-proxy.github_pr_comment_request", refusal)
    return true
  end
  if tostring(current_pr.state or ""):lower() ~= "open" then
    core.log_cas_decision("observe_pr", origin.proposal_id, state, "blocked|review-meta|reviewing", "reviewing", "refused(pr-closed)", "operator rereview requires an open PR")
    local refusal = core.build_operator_command_refusal_request(
      origin.repo,
      pr_number,
      command,
      "rereview requires an open PR",
      source_ref
    )
    core.log_raise("observe_pr", origin.proposal_id, "github-proxy.github_pr_comment_request", refusal)
    return true
  end
  if not core._is_git_sha(current_pr.head_sha) then
    core.log_cas_decision("observe_pr", origin.proposal_id, state, "blocked|review-meta|reviewing", "reviewing", "refused(head-missing)", "operator rereview requires a current PR head")
    local refusal = core.build_operator_command_refusal_request(
      origin.repo,
      pr_number,
      command,
      "rereview requires a current PR head",
      source_ref
    )
    core.log_raise("observe_pr", origin.proposal_id, "github-proxy.github_pr_comment_request", refusal)
    return true
  end

  local new_version = core.operator_rereview_version(state.version, current_pr.head_sha)
  local comment_request = core.build_operator_rereview_comment_request(
    origin.repo,
    pr_number,
    origin.proposal_id,
    new_version,
    command,
    source_ref
  )
  local reviewing_payload = core.build_devloop_reviewing_payload({
    proposal_id = origin.proposal_id,
    impl_version = new_version,
  }, pr_number, source_ref, new_version)
  core.log_cas_decision("observe_pr", origin.proposal_id, state, "blocked|review-meta|reviewing", "reviewing", "applied(operator-rereview)", "trusted operator command requested rereview")
  core.log_apply("observe_pr", origin.proposal_id, "reviewing", new_version, { add = {}, remove = {} }, {
    "github-proxy.github_pr_comment_request",
    "devloop_reviewing",
  })
  core.log_raise("observe_pr", origin.proposal_id, "github-proxy.github_pr_comment_request", comment_request)
  core.log_raise("observe_pr", origin.proposal_id, "devloop_reviewing", reviewing_payload)
  maybe_label_hints(origin, pr_number, current_pr, { state = "reviewing", version = new_version }, source_ref)
  return true
end

local function direct_opened_matches_origin(pr, origin, current_pr)
  if pr.source ~= "direct-opened" then
    return false
  end
  return tostring(pr.proposal_id or "") == tostring(origin.proposal_id or "")
    and tostring(pr.issue_number or "") == tostring(origin.issue_number or "")
    and tostring(pr.impl_version or "") == tostring(origin.impl_version or "")
    and tostring(pr.branch or "") == tostring(origin.branch or "")
    and tostring(pr.head_sha or "") == tostring(current_pr.head_sha or "")
    and tostring(pr.base_branch or "") == tostring(origin.base_branch or "")
end

local function liveness_timeout_state(state)
  local row = core.restart_transition_row(state and state.state)
  if row == nil or core.liveness_timeout_due(row, state, now()) ~= true then
    return state
  end
  return {
    state = state.state,
    version = core.next_liveness_timeout_version(row, state),
    proposal_id = state.proposal_id,
    stage_rank = state.stage_rank,
    marker_created_at = state.marker_created_at,
  }
end

function pipeline(event)
  local pr = pr_context(event)
  local raw = event.payload or {}
  if pr == nil then
    core.log_entry("observe_pr", event, "unknown", core.payload_field(raw, "dedup_key"))
    core.log_cas_decision("observe_pr", "unknown", { state = nil, version = nil }, "pr-open", "reviewing", "skip-foreign(pr)", "unsupported event payload")
    return
  end

  core.log_entry("observe_pr", event, "unknown", pr.dedup_key)
  core.assert_trusted_bot_configured()
  local branches = core.branch_config()
    local pr_view = core.fetch_pr_view_origin(pr.repo, pr.number, pr.updated_at)
  if pr_view.exit_code ~= 0 then
    error("github-devloop: gh pr origin view failed: " .. tostring(pr_view.stderr))
  end

  local current_pr = core.parse_pr_view_origin(pr_view.stdout)
  local origin, has_issue_origin = origin_from_pr(pr.repo, pr.number, current_pr)
  if origin.branch == nil or origin.base_branch == nil then
    core.log_cas_decision("observe_pr", origin.proposal_id, { state = nil, version = nil }, "pr-open", "reviewing", "skip-foreign(pr)", "PR branch facts missing")
    return
  end
  local ok, reason = origin_matches_pr(origin, current_pr, pr.repo, branches, false)
  if not ok then
    core.log_cas_decision("observe_pr", origin.proposal_id, { state = nil, version = nil }, "pr-open", "reviewing", "skip-foreign(" .. reason .. ")", "PR origin mismatch")
    return
  end

  local source_ref = pr_source_ref(pr.repo, pr.number)
  local lock_key = core.transition_lock_key(origin.proposal_id)
  if lock_key == nil then
    core.log_cas_decision("observe_pr", origin.proposal_id, { state = nil, version = nil }, "pr-open", "reviewing", "skip-foreign(proposal_id)", "no transition lock key")
    return
  end

  with_lock(lock_key, function()
    local state = core.current_entity_state(current_pr.comments, origin.proposal_id)
    local issue_current = nil
    local function issue_claim_ok()
      if issue_current == nil then
        issue_current = issue_claim_for_origin(origin)
      end
      return core.verify_pr_review_issue_claim("observe_pr", origin.repo, origin.issue_number, issue_current, origin.proposal_id)
    end
    local merge_gate_feedback = nil
    if state.state == "reviewing" and origin.issue_number ~= nil then
      merge_gate_feedback = core.merge_gate_fix_fact(current_pr.comments, origin.proposal_id, core.next_fix_version(state.version))
    end
    if merge_gate_feedback ~= nil then
      if not issue_claim_ok() then
        return
      end
      if issue_current == nil or issue_current.comments == nil then
        issue_current = issue_reviewing_for_origin(origin)
      end
      local issue_comments = issue_current and issue_current.comments or issue_comments_for_origin(origin)
      local issue_state = core.current_entity_state(issue_comments, origin.proposal_id)
      if issue_state.state == "fixing" then
        core.log_cas_decision("observe_pr", origin.proposal_id, issue_state, "fixing", "fixing", "applied(issue-fixing-replay)", "issue marker is fixing while PR marker is still reviewing")
        if raise_current_state(origin, pr.number, current_pr, issue_state, source_ref, { comments = issue_comments }) then
          maybe_label_hints(origin, pr.number, current_pr, issue_state, source_ref)
        end
        return
      end
    end
    if core.operator_command_fact(current_pr.comments, "rereview") ~= nil and not issue_claim_ok() then
      return
    end
    if maybe_apply_rereview_command(origin, pr.number, current_pr, state, source_ref) then
      return
    end
    if state.state ~= nil and state.state ~= "pr-open" then
      local replay_state = pr.source == "poll" and raw.source == "liveness-scan" and liveness_timeout_state(state) or state
      core.log_cas_decision("observe_pr", origin.proposal_id, state, "reviewing", state.state, "skip-idempotent(already at to_state)", state.state .. " marker visible on PR")
      if replay_state.state == "reviewing" and not issue_claim_ok() then
        return
      end
      if raise_current_state(origin, pr.number, current_pr, replay_state, source_ref, issue_current) then
        maybe_label_hints(origin, pr.number, current_pr, replay_state, source_ref)
      end
      return
    end

    local direct_opened = direct_opened_matches_origin(pr, origin, current_pr)
    if pr.source == "direct-opened" and not direct_opened then
      core.log_cas_decision("observe_pr", origin.proposal_id, state, "pr-open", "reviewing", "skip-stale(direct-opened-fact-mismatch)", "direct PR-opened event does not match canonical PR origin")
      return
    end
    local transition = core.versioned_transition_status(state, { "pr-open", "unmanaged" }, "reviewing", origin.impl_version)
    if has_issue_origin and transition == "pending" then
      if direct_opened then
        transition = "apply"
      else
        core.log_cas_decision("observe_pr", origin.proposal_id, state, "pr-open", "reviewing", core.cas_outcome(state, transition, origin.impl_version), "reviewing PR marker not yet visible")
        return
      end
    end
    if state.state == "pr-open" and tostring(state.version or "") ~= tostring(origin.impl_version or "") then
      core.log_cas_decision("observe_pr", origin.proposal_id, state, "pr-open", "reviewing", "skip-stale(version-mismatch)", "PR-open marker version does not match PR origin")
      return
    end
    if transition ~= "apply" and transition ~= "idempotent" then
      core.log_cas_decision("observe_pr", origin.proposal_id, state, "pr-open", "reviewing", core.cas_outcome(state, transition, origin.impl_version), "current PR state cannot advance to reviewing")
      return
    end
    if tostring(current_pr.state or ""):lower() ~= "open" then
      core.log_cas_decision("observe_pr", origin.proposal_id, state, "pr-open", "reviewing", "skip-stale(pr-closed)", "re-derived PR is not open")
      return
    end
    if not issue_claim_ok() then
      return
    end
    core.log_cas_decision("observe_pr", origin.proposal_id, state, "pr-open", "reviewing", "applied", "writing PR-local reviewing marker")
    local comment_request = core.build_reviewing_comment_request(origin.repo, origin.issue_number, origin, pr.number, source_ref)
    local reviewing_payload = core.build_devloop_reviewing_payload(origin, pr.number, source_ref)
    local raised = {
      "github-proxy.github_pr_comment_request",
      "devloop_reviewing",
    }
    core.log_apply("observe_pr", origin.proposal_id, "reviewing", origin.impl_version, { add = {}, remove = {} }, raised)
    core.log_raise("observe_pr", origin.proposal_id, "github-proxy.github_pr_comment_request", comment_request)
    core.log_raise("observe_pr", origin.proposal_id, "devloop_reviewing", reviewing_payload)
    maybe_label_hints(origin, pr.number, current_pr, { state = "reviewing", version = origin.impl_version }, source_ref)
  end)
end

pipeline = core.wrap_pipeline_failure("observe_pr", pipeline)

return M
