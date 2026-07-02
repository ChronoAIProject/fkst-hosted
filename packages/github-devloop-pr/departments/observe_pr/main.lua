local devloop_base = require("devloop.base")
local entity_lib = require("devloop.entity")
local m_claims = require("devloop.claims")
local requests_labels = require("devloop.requests.labels")
local requests_review = require("devloop.requests.review")
local parsers_pr = require("devloop.parsers.pr")
local parsers_issue = require("devloop.parsers.issue")
local convergence_shared = require("devloop.convergence.shared")
local check_runs = require("forge.github.check_runs")
local queue = require("devloop.queue")
local transition_version = require("contract.transition_version")
local m_facts = require("devloop.markers.facts")
local core, saga, replay_fields = require("core"), require("workflow.saga"), require("devloop.replay_fields")
local forge_validators = require("devloop.forge_validators")
local operator_commands = require("devloop.operator_commands")
local decompose_lib = require("devloop.decompose")
local replayer = require("devloop.replayer")
local config = require("devloop.config")
local conv_rounds = require("devloop.convergence.rounds")
local v_pr = require("devloop.validators.pr")
local devloop_entity_view = require("devloop.github_proxy_entity_view")

local M = {}

local spec = {
  consumes = { "github-proxy.github_entity_changed", "devloop_observe_pr" },
  produces = {
    "github-proxy.github_issue_label_request",
    "github-proxy.github_pr_comment_request",
    "github-devloop-decompose.devloop_decompose",
    -- devloop_reviewing is emitted only after github_comment_written via comment_handoff.
    "devloop_fixing",
    "devloop_review_meta",
    "devloop_merge_ready",
    "devloop_review_reconcile",
    "devloop_timeout_reconcile",
  },
  stall_window = "30s",
  retry = { max_attempts = 12, base = "5s", cap = "30s" },
}

local function pr_source_ref(repo, pr_number)
  return entity_lib.pr_source_ref(repo, pr_number)
end

local function pr_context(event)
  local payload = event.payload or {}
  if v_pr.is_supported_pr(core, payload) then
    return {
      source = "poll",
      repo = payload.repo,
      number = payload.number,
      dedup_key = payload.dedup_key,
      source_ref = payload.source_ref,
    }
  end
  return nil
end

local function origin_from_pr(repo, pr_number, current_pr)
  local origin = m_facts.pr_origin_fact(core, current_pr.comments)
  if origin ~= nil then
    return origin, true
  end
  return entity_lib.pr_native_origin(repo, pr_number, current_pr), false
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

local function origin_base_matches_current_pr(origin, current_pr)
  return tostring(current_pr.base_ref_name or "") == tostring(origin.base_branch)
end

local function origin_base_matches_integration(origin, branches)
  return origin.base_branch ~= nil
    and tostring(origin.base_branch or "") == tostring(branches.integration)
end

local function maybe_pr_label_hint(origin, pr_number, current_pr, state, source_ref)
  if state.state == nil then
    return
  end
  local add_labels, remove_labels = core.state_label_reconcile_changes(current_pr.labels, state.state)
  local label_request = core.build_reconcile_pr_state_label_request(origin.repo, origin.issue_number, pr_number, origin.proposal_id, state.state, state.version, source_ref, current_pr.labels)
  if (#add_labels == 0 and #remove_labels == 0) or not core.pr_state_label_request_guard_visible(current_pr.comments, label_request) then
    return
  end
  core.log_apply("observe_pr", origin.proposal_id, state.state, state.version, { add = add_labels, remove = remove_labels }, {
    "github-proxy.github_issue_label_request",
  })
  core.log_raise("observe_pr", origin.proposal_id, "github-proxy.github_issue_label_request", label_request)
end

local function maybe_label_hints(origin, pr_number, current_pr, state, pr_source_ref_value)
  maybe_pr_label_hint(origin, pr_number, current_pr, state, pr_source_ref_value)
end

local function issue_reviewing_for_origin(origin)
  if origin.issue_number == nil then
    return nil
  end
  local issue_view = core.gh_issue_view_reviewing(origin.repo, origin.issue_number, 30)
  if issue_view.exit_code ~= 0 then
    error("github-devloop: gh issue reviewing view failed: " .. tostring(issue_view.stderr))
  end
  return parsers_issue.parse_issue_view_reviewing(core, issue_view.stdout)
end

local function issue_claim_for_origin(origin)
  if origin.issue_number == nil then
    return nil
  end
  return m_claims.read_current_issue_ownership(core, origin.repo, origin.issue_number)
end

local function replay_pr_local_state(origin, pr_number, current_pr, state, source_ref)
  if state.state == "blocked" and decompose_lib.decomposed_fact(core, current_pr.comments, origin.proposal_id, state.version, pr_number) == nil then
    core.log_cas_decision("observe_pr", origin.proposal_id, state, "blocked", "decomposed", "skip-foreign(decomposed)", "decomposed marker is not visible")
    return false
  end
  return replayer.replay_from_table(core, "observe_pr", {
    repo = origin.repo,
    number = origin.issue_number,
    source_ref = origin.issue_number ~= nil and entity_lib.issue_source_ref(origin.repo, origin.issue_number) or source_ref,
  }, state, replay_fields.restart_transition_row(core.restart_transition_table(), state.state), {
    proposal_id = origin.proposal_id,
    current = { comments = current_pr.comments or {} },
    current_pr = current_pr,
    snapshot = {
      comments = current_pr.comments or {},
      prs = { { number = pr_number, current = current_pr } },
      state = state,
    },
    link = {
      proposal_id = origin.proposal_id,
      pr_number = pr_number,
      branch = origin.branch,
      impl_version = origin.impl_version,
      base_branch = origin.base_branch,
    },
    source_ref = source_ref,
    feedback = core.fixing_replay_feedback_fact(current_pr.comments, origin.proposal_id, state.version), fix_feedback = core.fixing_replay_feedback_fact(current_pr.comments, origin.proposal_id, state.version),
  })
end

local function is_stalled_reviewing(current_pr, origin, pr_number, state)
  if state.state ~= "reviewing" or not forge_validators.is_git_sha(current_pr.head_sha) then
    return false
  end
  local review_proposal_id = devloop_base.pr_review_proposal_id(origin.repo, pr_number, state.version, current_pr.head_sha)
  local review_version = transition_version.safe_version_segment(state.version)
  local sr_digest = convergence_shared.source_ref_digest(entity_lib.pr_source_ref(origin.repo, pr_number))
  local facts = conv_rounds.review_converge_round_facts(core,
    current_pr.comments,
    review_proposal_id,
    origin.proposal_id,
    review_version,
    current_pr.head_sha,
    sr_digest
  )
  local round = conv_rounds.max_converge_round(core, facts)
  return conv_rounds.is_true_stall(core, facts, round)
end

local function maybe_apply_rereview_command(origin, pr_number, current_pr, state, source_ref)
  local command = operator_commands.operator_command_fact(core, current_pr.comments, "rereview")
  if command == nil then
    return false
  end
  if operator_commands.has_operator_command_response(core, current_pr.comments, command) then
    core.log_cas_decision("observe_pr", origin.proposal_id, state, "blocked|review-meta|reviewing", "reviewing", "skip-idempotent(command-response-visible)", "operator command response marker is already visible")
    return false
  end
  if state.state ~= "blocked" and state.state ~= "review-meta" and state.state ~= "reviewing" then
    core.log_cas_decision("observe_pr", origin.proposal_id, state, "blocked|review-meta|reviewing", "reviewing", "refused(invalid-state)", "operator rereview precondition failed")
    local refusal = operator_commands.build_operator_command_refusal_request(
      core,
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
    local refusal = operator_commands.build_operator_command_refusal_request(
      core,
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
    local refusal = operator_commands.build_operator_command_refusal_request(
      core,
      origin.repo,
      pr_number,
      command,
      "rereview requires an open PR",
      source_ref
    )
    core.log_raise("observe_pr", origin.proposal_id, "github-proxy.github_pr_comment_request", refusal)
    return true
  end
  if not forge_validators.is_git_sha(current_pr.head_sha) then
    core.log_cas_decision("observe_pr", origin.proposal_id, state, "blocked|review-meta|reviewing", "reviewing", "refused(head-missing)", "operator rereview requires a current PR head")
    local refusal = operator_commands.build_operator_command_refusal_request(
      core,
      origin.repo,
      pr_number,
      command,
      "rereview requires a current PR head",
      source_ref
    )
    core.log_raise("observe_pr", origin.proposal_id, "github-proxy.github_pr_comment_request", refusal)
    return true
  end

  local new_version = operator_commands.operator_rereview_version(core, state.version, current_pr.head_sha)
  local comment_request = requests_review.build_operator_rereview_comment_request(core,
    origin.repo,
    pr_number,
    origin.proposal_id,
    new_version,
    command,
    source_ref
  )
  core.log_cas_decision("observe_pr", origin.proposal_id, state, "blocked|review-meta|reviewing", "reviewing", "applied(operator-rereview)", "trusted operator command requested rereview")
  core.log_apply("observe_pr", origin.proposal_id, "reviewing", new_version, { add = {}, remove = {} }, {
    "github-proxy.github_pr_comment_request",
  })
  core.log_raise("observe_pr", origin.proposal_id, "github-proxy.github_pr_comment_request", comment_request)
  maybe_label_hints(origin, pr_number, current_pr, { state = "reviewing", version = new_version }, source_ref)
  return true
end

local function maybe_liveness_timeout(origin, pr_number, current_pr, state, source_ref, issue_current)
  local row = replay_fields.restart_transition_row(core.restart_transition_table(), state and state.state)
  if not core.restart_row_observable_on(row, "pr") then
    return false
  end
  local issue_source_ref = origin.issue_number ~= nil and entity_lib.issue_source_ref(origin.repo, origin.issue_number) or source_ref
  local head_sha = current_pr and current_pr.head_sha
  return core.maybe_timeout_redrive_from_table("observe_pr", {
    repo = origin.repo,
    number = origin.issue_number,
    source_ref = issue_source_ref,
    _replay_issue_comments = issue_current and issue_current.comments or nil,
  }, state, row, {
    proposal_id = origin.proposal_id,
    current = { comments = issue_current and issue_current.comments or {} },
    current_pr = current_pr,
    link = {
      proposal_id = origin.proposal_id,
      pr_number = pr_number,
      branch = origin.branch,
      impl_version = origin.impl_version,
      base_branch = origin.base_branch,
    },
    source_ref = source_ref,
    head_sha = head_sha,
    fresh_current_state = state,
    review_proposal_id = state and state.state == "reviewing" and forge_validators.is_git_sha(head_sha)
      and devloop_base.pr_review_proposal_id(origin.repo, pr_number, state.version, head_sha)
      or nil,
  })
end

local function build_conflict_review_fact(origin, pr_number, current_pr, version, reason)
  local head_sha = tostring(current_pr.head_sha or "")
  if not forge_validators.is_git_sha(head_sha) then
    return nil, "head-missing"
  end
  return {
    review_proposal_id = devloop_base.pr_review_proposal_id(origin.repo, pr_number, version, head_sha),
    review_dedup_key = "observe-pr-conflict/" .. tostring(origin.proposal_id) .. "/" .. tostring(version) .. "/" .. tostring(pr_number),
    reviewed_head_sha = head_sha,
    gate_failure_excerpt = reason,
  }, "ok"
end

local function maybe_redrive_not_mergeable_pr(origin, pr_number, current_pr, state, source_ref, issue_current)
  local row = replay_fields.restart_transition_row(core.restart_transition_table(), state and state.state)
  local recovery = row and row.pr_recovery and row.pr_recovery.not_mergeable or nil
  if recovery == nil then
    return false
  end
  if tostring(current_pr.state or ""):lower() ~= "open" then
    return false
  end
  local mergeable, reason = check_runs.pr_mergeable(current_pr)
  if mergeable or not check_runs.is_not_mergeable_reason(reason) then
    return false
  end
  if core.version_fix_round(state.version) >= config.max_fix_rounds(core) then
    core.log_cas_decision("observe_pr", origin.proposal_id, state, state.state, recovery.to_state, "skip-idempotent(fix-loop-max-rounds)", reason)
    return false
  end
  local fix_version = core.next_fix_version(state.version)
  local visible_state = require("devloop.entity").current_entity_state(core, current_pr.comments, origin.proposal_id)
  if visible_state.state == "fixing" and tostring(visible_state.version or "") == tostring(fix_version) then
    core.log_cas_decision("observe_pr", origin.proposal_id, state, state.state, recovery.to_state, "skip-idempotent(already at to_state)", reason)
    return true
  end
  local review_fact, fact_reason = build_conflict_review_fact(origin, pr_number, current_pr, state.version, reason)
  if review_fact == nil then
    core.log_cas_decision("observe_pr", origin.proposal_id, state, state.state, recovery.to_state, "retry-pending(" .. fact_reason .. ")", reason)
    return false
  end
  review_fact.fix_version = fix_version
  local comment_origin = {
    proposal_id = origin.proposal_id,
    pr_number = pr_number,
    version = state.version,
    review_proposal_id = review_fact.review_proposal_id,
    review_dedup_key = review_fact.review_dedup_key,
    reviewed_head_sha = review_fact.reviewed_head_sha,
    dedup_key = tostring(state.version) .. "/observe-pr-conflict",
  }
  local comment_request = requests_review.build_merge_gate_fix_comment_request(core,
    origin.repo,
    origin.issue_number,
    comment_origin,
    fix_version,
    reason,
    nil,
    source_ref,
    nil,
    {
      gate_failure_excerpt = reason,
    }
  )
  local label_request = origin.issue_number ~= nil and requests_labels.build_state_label_request(core,
    origin.repo,
    origin.issue_number,
    "fixing",
    tostring(state.version) .. "/observe-pr-conflict/label/fixing",
    entity_lib.issue_source_ref(origin.repo, origin.issue_number)
  ) or nil
  core.log_cas_decision("observe_pr", origin.proposal_id, state, state.state, recovery.to_state, "applied(not-mergeable)", reason)
  local raised = {
    "github-proxy.github_pr_comment_request",
  }
  if label_request ~= nil then
    table.insert(raised, "github-proxy.github_issue_label_request")
  end
  core.log_apply("observe_pr", origin.proposal_id, "fixing", fix_version, { add = { "fkst-dev:fixing" }, remove = {} }, raised)
  core.log_raise("observe_pr", origin.proposal_id, "github-proxy.github_pr_comment_request", comment_request)
  if label_request ~= nil then
    core.log_raise("observe_pr", origin.proposal_id, "github-proxy.github_issue_label_request", label_request)
  end
  maybe_label_hints(origin, pr_number, current_pr, { state = "fixing", version = fix_version }, source_ref)
  return true
end

local function maybe_block_unmanaged_base(pr, origin, current_pr, branches, source_ref)
  if origin.issue_number == nil then
    core.log_cas_decision("observe_pr", origin.proposal_id, { state = nil, version = nil }, "pr-open", "blocked", "skip-not-owned", "backing issue is absent")
    return true
  end
  local lock_key = entity_lib.transition_lock_key(origin.proposal_id)
  if lock_key == nil then
    core.log_cas_decision("observe_pr", origin.proposal_id, { state = nil, version = nil }, "pr-open", "blocked", "skip-foreign(proposal_id)", "no transition lock key")
    return true
  end

  with_lock(lock_key, function()
    local state = require("devloop.entity").current_entity_state(core, current_pr.comments, origin.proposal_id)
    local issue_current = issue_claim_for_origin(origin)
    if not m_claims.verify_pr_review_issue_claim(core, "observe_pr", origin.repo, origin.issue_number, issue_current, origin.proposal_id) then
      return
    end
    if state.state == "blocked" then
      core.log_cas_decision("observe_pr", origin.proposal_id, state, "pr-open", "blocked", "skip-idempotent(already at to_state)", "blocked marker visible on PR")
      maybe_label_hints(origin, pr.number, current_pr, state, source_ref)
      return
    end
    if state.state ~= "pr-open" then
      core.log_cas_decision("observe_pr", origin.proposal_id, state, "pr-open", "blocked", "skip-stale(state-mismatch)", "PR is not in pr-open state")
      return
    end
    if tostring(state.version or "") ~= tostring(origin.impl_version or "") then
      core.log_cas_decision("observe_pr", origin.proposal_id, state, "pr-open", "blocked", "skip-stale(version-mismatch)", "PR-open marker version does not match PR origin")
      return
    end
    if tostring(current_pr.state or ""):lower() ~= "open" then
      core.log_cas_decision("observe_pr", origin.proposal_id, state, "pr-open", "blocked", "skip-stale(pr-closed)", "re-derived PR is not open")
      return
    end

    local blocked_version = requests_review.pr_base_unmanaged_blocked_version(core, origin.impl_version)
    local blocked_state = {
      state = "blocked",
      version = blocked_version,
      proposal_id = origin.proposal_id,
    }
    local comment_request = requests_review.build_pr_base_unmanaged_comment_request(core, origin.repo, pr.number, origin, branches.integration, source_ref)
    core.log_cas_decision("observe_pr", origin.proposal_id, state, "pr-open", "blocked", "applied(pr-base-unmanaged)", "self-claimed PR base is not managed by this instance")
    core.log_apply("observe_pr", origin.proposal_id, "blocked", blocked_version, { add = { "fkst-dev:blocked" }, remove = {} }, {
      "github-proxy.github_pr_comment_request",
    })
    core.log_raise("observe_pr", origin.proposal_id, "github-proxy.github_pr_comment_request", comment_request)
    maybe_pr_label_hint(origin, pr.number, current_pr, blocked_state, source_ref)
  end)
  return true
end

local function process_pr_event(event)
  local pr = pr_context(event)
  local raw = event.payload or {}
  if pr == nil then
    core.log_entry("observe_pr", event, "unknown", core.payload_field(raw, "dedup_key"))
    core.log_cas_decision("observe_pr", "unknown", { state = nil, version = nil }, "pr-open", "reviewing", "skip-foreign(pr)", "unsupported event payload")
    return
  end

  core.log_entry("observe_pr", event, "unknown", pr.dedup_key)
  devloop_base.assert_trusted_bot_configured()
  local branches = config.branch_config(core)
  local pr_view = devloop_entity_view.fetch_pr_view_origin(pr.repo, pr.number, pr.updated_at)
  if pr_view.exit_code ~= 0 then
    error("github-devloop: gh pr origin view failed: " .. tostring(pr_view.stderr))
  end

  local current_pr = parsers_pr.parse_pr_view_origin(core, pr_view.stdout)
  local origin, has_issue_origin = origin_from_pr(pr.repo, pr.number, current_pr)
  if origin.branch == nil or origin.base_branch == nil then
    core.log_cas_decision("observe_pr", origin.proposal_id, { state = nil, version = nil }, "pr-open", "reviewing", "skip-foreign(pr)", "PR branch facts missing")
    return
  end
  local ok, reason = origin_matches_pr(origin, current_pr, pr.repo, branches, false)
  if not ok then
    if reason == "base"
      and origin_base_matches_current_pr(origin, current_pr)
      and not origin_base_matches_integration(origin, branches) then
      local source_ref = pr_source_ref(pr.repo, pr.number)
      if maybe_block_unmanaged_base(pr, origin, current_pr, branches, source_ref) then
        return
      end
    end
    core.log_cas_decision("observe_pr", origin.proposal_id, { state = nil, version = nil }, "pr-open", "reviewing", "skip-foreign(" .. reason .. ")", "PR origin mismatch")
    return
  end

  local source_ref = pr_source_ref(pr.repo, pr.number)
  local lock_key = entity_lib.transition_lock_key(origin.proposal_id)
  if lock_key == nil then
    core.log_cas_decision("observe_pr", origin.proposal_id, { state = nil, version = nil }, "pr-open", "reviewing", "skip-foreign(proposal_id)", "no transition lock key")
    return
  end

  with_lock(lock_key, function()
    local state = require("devloop.entity").current_entity_state(core, current_pr.comments, origin.proposal_id)
    local issue_current = issue_claim_for_origin(origin)
    if not m_claims.verify_pr_review_issue_claim(core, "observe_pr", origin.repo, origin.issue_number, issue_current, origin.proposal_id) then
      return
    end
    local merge_gate_feedback = nil
    if state.state == "reviewing" and origin.issue_number ~= nil then
      merge_gate_feedback = m_facts.merge_gate_fix_fact(core, current_pr.comments, origin.proposal_id, core.next_fix_version(state.version))
    end
    if merge_gate_feedback ~= nil then
      if issue_current == nil or issue_current.comments == nil then
        issue_current = issue_reviewing_for_origin(origin)
      end
      local issue_comments = issue_current and issue_current.comments or {}
      local issue_state = require("devloop.entity").current_entity_state(core, issue_comments, origin.proposal_id)
      if issue_state.state == "fixing" then
        core.log_cas_decision("observe_pr", origin.proposal_id, issue_state, "fixing", "fixing", "applied(issue-fixing-replay)", "issue marker is fixing while PR marker is still reviewing")
        maybe_label_hints(origin, pr.number, current_pr, issue_state, source_ref)
        return
      end
    end
    if maybe_apply_rereview_command(origin, pr.number, current_pr, state, source_ref) then
      return
    end
    if maybe_redrive_not_mergeable_pr(origin, pr.number, current_pr, state, source_ref, issue_current) then
      return
    end
    if state.state ~= nil and state.state ~= "pr-open" then
      if pr.source == "poll"
        and raw.source == "liveness-scan"
        and maybe_liveness_timeout(origin, pr.number, current_pr, state, source_ref, issue_current) then
        return
      end
      local replay_state = state
      core.log_cas_decision("observe_pr", origin.proposal_id, state, "reviewing", state.state, "skip-idempotent(already at to_state)", state.state .. " marker visible on PR")
      local replayed = replay_pr_local_state(origin, pr.number, current_pr, replay_state, source_ref)
      if replayed then
        maybe_label_hints(origin, pr.number, current_pr, replay_state, source_ref)
      elseif replay_state.state == "blocked" or replay_state.state == "merged" then
        maybe_label_hints(origin, pr.number, current_pr, replay_state, source_ref)
      end
      return
    end

    local transition = core.versioned_transition_status(state, { "pr-open", "unmanaged" }, "reviewing", origin.impl_version)
    if has_issue_origin and transition == "pending" then
      core.log_cas_decision("observe_pr", origin.proposal_id, state, "pr-open", "reviewing", core.cas_outcome(state, transition, origin.impl_version), "reviewing PR marker not yet visible")
      return
    end
    if state.state == "pr-open" and tostring(state.version or "") ~= tostring(origin.impl_version or "") then
      core.log_cas_decision("observe_pr", origin.proposal_id, state, "pr-open", "reviewing", "skip-stale(version-mismatch)", "PR-open marker version does not match PR origin")
      return
    end
    if state.state == "pr-open" and tostring(current_pr.state or ""):lower() ~= "open" then
      replay_pr_local_state(origin, pr.number, current_pr, state, source_ref)
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
    if maybe_redrive_not_mergeable_pr(origin, pr.number, current_pr, state, source_ref, issue_current) then
      return
    end
    core.log_cas_decision("observe_pr", origin.proposal_id, state, "pr-open", "reviewing", "applied", "writing PR-local reviewing marker")
    local comment_request = requests_review.build_reviewing_comment_request(core, origin.repo, origin.issue_number, origin, pr.number, source_ref)
    local raised = {
      "github-proxy.github_pr_comment_request",
    }
    core.log_apply("observe_pr", origin.proposal_id, "reviewing", origin.impl_version, { add = {}, remove = {} }, raised)
    core.log_raise("observe_pr", origin.proposal_id, "github-proxy.github_pr_comment_request", comment_request)
  end)
end

return saga.department(spec, { done = function() return false end, act = function(event)
  queue.dispatch_consumed_queue("observe_pr", spec, event, {
    ["github-proxy.github_entity_changed"] = process_pr_event,
    devloop_observe_pr = process_pr_event,
  }, "github-devloop-pr")
end, wrap = core.wrap_pipeline_failure, name = "observe_pr" })
