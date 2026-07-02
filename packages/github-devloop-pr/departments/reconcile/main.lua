local entity_lib = require("devloop.entity")
local devloop_base = require("devloop.base")
local base_ids = require("devloop.base_ids")
local m_claims = require("devloop.claims")
local requests_labels = require("devloop.requests.labels")
local parsers_misc = require("devloop.parsers.misc")
local parsers_pr = require("devloop.parsers.pr")
local parsers_issue = require("devloop.parsers.issue")
local m_facts = require("devloop.markers.facts")
local m_mgw = require("devloop.merge_gate_wait")
local core, replay_fields = require("core"), require("devloop.replay_fields")
local check_runs = require("forge.github.check_runs")
local transition_version = require("contract.transition_version")

local saga = require("workflow.saga")
local forge_validators = require("devloop.forge_validators")
local conv_reconcile = require("devloop.convergence.reconcile")
local conv_attempts = require("devloop.convergence.attempts")

local spec = {
  consumes = { "devloop_review_reconcile", "devloop_fix_reconcile", "devloop_timeout_reconcile" },
  produces = {
    "github-proxy.github_issue_comment_request",
    "github-proxy.github_pr_comment_request",
    "github-proxy.github_issue_label_request",
  },
  stall_window = "2m",
}

local function emit_blocked_reconcile(kind, proposal_id, state, version, action, reason, comment_request, label_request, comment_queue)
  local add_labels, remove_labels = core.state_label_changes("blocked")
  local queue = comment_queue or "github-proxy.github_issue_comment_request"
  core.log_cas_decision("reconcile", proposal_id, state, kind, "blocked", "applied", reason)
  core.log_apply("reconcile", proposal_id, "blocked", version, { add = add_labels, remove = remove_labels }, {
    queue,
    "github-proxy.github_issue_label_request",
  })
  core.log_raise("reconcile", proposal_id, queue, comment_request)
  if label_request ~= nil then
    core.log_raise("reconcile", proposal_id, "github-proxy.github_issue_label_request", label_request)
  end
end

local function build_timeout_reconcile_pr_comment_request(repo, pr_number, reconcile, action, reason, version, fields)
  local marker = conv_reconcile.timeout_reconcile_marker(core, reconcile.proposal_id, reconcile.issue_version, reconcile.state, reconcile.round, action, fields)
  local state_marker = core.state_marker(reconcile.proposal_id, "blocked", version)
  return entity_lib.build_entity_comment_request({
    kind = "pr",
    repo = repo,
    number = pr_number,
  }, "github-devloop timeout reconcile action: " .. tostring(action)
    .. "\n\nReason:\n" .. tostring(reason or "")
    .. "\n\nStructured WHY:\n" .. conv_reconcile.timeout_reconcile_reason_body(core, fields or {})
    .. "\n\n" .. state_marker .. "\n" .. marker
    .. "\n" .. "⟦AI:FKST⟧", base_ids.dedup_key({
    "timeout-reconcile",
    "pr-comment",
    tostring(reconcile.dedup_key),
  }), reconcile.source_ref)
end

local function merge_wait_timeout_reason_class(reconcile, state, comments, current_pr)
  if reconcile.state ~= "merge-ready" and reconcile.state ~= "merging" then
    return "state-output-obligation-timeout"
  end
  local _, pr_number = devloop_base.parse_pr_source_ref(reconcile.source_ref)
  local head_sha = current_pr and current_pr.head_sha or nil
  if pr_number == nil or not forge_validators.is_git_sha(head_sha) then
    return "state-output-obligation-timeout"
  end
  local wait = m_mgw.merge_gate_wait_fact(core, comments, reconcile.proposal_id, state.version, pr_number, head_sha)
  if wait == nil then
    return "state-output-obligation-timeout"
  end
  local reason_class = core.merge_gate_reason_class(wait.reason)
  local wait_kind = tostring(wait.kind or "")
  if parsers_misc.is_ci_red_reason(core, reason_class) or check_runs.is_not_mergeable_reason(reason_class) then
    return "state-output-obligation-timeout"
  end
  if reason_class == "ci-wait"
    or parsers_misc.is_ci_wait_reason(core, reason_class)
    or wait_kind == "CI_WAIT"
    or wait_kind == "CHECKS_PENDING"
    or wait_kind == "CI_UNKNOWN"
    or wait_kind == "EXTERNAL_CI_RED"
    or wait_kind == "INTEGRATION_RED" then
    return "external-ci-wait-expired"
  end
  return "state-output-obligation-timeout"
end

local function timeout_reconcile_needs_pr_surface(state_name)
  return state_name == "pr-open"
    or state_name == "reviewing"
    or state_name == "fixing"
    or state_name == "review-meta"
    or state_name == "merge-ready"
    or state_name == "merging"
end

local function command_indicates_not_found(result)
  local stderr = tostring(result and result.stderr or ""):lower()
  return stderr:find("404", 1, true) ~= nil
    or stderr:find("not found", 1, true) ~= nil
end

local function load_timeout_issue_surface(repo, issue_number, proposal_id, state_name)
  local view = core.gh_issue_view_loop(repo, issue_number, 30)
  if view.exit_code ~= 0 then
    error("github-devloop: timeout-reconcile-issue-view-failed: " .. tostring(view.stderr))
  end
  local current_issue = parsers_issue.parse_issue_view_loop(core, view.stdout)
  local issue_state = require("devloop.entity").current_entity_state(core, current_issue.comments, proposal_id)
  if timeout_reconcile_needs_pr_surface(state_name) then
    local snapshot = core.linked_pr_surface_snapshot(repo, proposal_id, current_issue.comments)
    local current_pr = nil
    local link = m_facts.pr_link_fact(core, snapshot.comments, proposal_id)
    if link ~= nil then
      for _, item in ipairs(snapshot.prs or {}) do
        if tostring(item.number or "") == tostring(link.pr_number or "") then
          current_pr = item.current
          break
        end
      end
    end
    snapshot.state = issue_state
    return current_issue, current_pr, snapshot.comments, snapshot
  end
  return current_issue, nil, current_issue.comments, nil
end

local function pipeline_review(event)
  local reconcile = event.payload or {}
  if not conv_reconcile.is_supported_review_reconcile(core, reconcile) then
    core.log_entry("reconcile", event, "unknown", core.payload_field(reconcile, "dedup_key"))
    core.log_cas_decision("reconcile", "unknown", { state = nil, version = nil }, "reviewing", "blocked", "skip-foreign(proposal_id)", "unsupported event payload")
    return
  end

  core.log_entry("reconcile", event, reconcile.proposal_id, reconcile.dedup_key)
  local entity = entity_lib.parse_entity_proposal_id(reconcile.proposal_id)
  if entity == nil then
    core.log_cas_decision("reconcile", reconcile.proposal_id, { state = nil, version = nil }, "reviewing", "blocked", "skip-foreign(proposal_id)", "proposal_id is outside github-devloop")
    return
  end
  local repo = entity.repo
  local issue_number = entity.issue_number
  local _, pr_number = devloop_base.parse_pr_source_ref(reconcile.source_ref)
  if pr_number == nil then
    pr_number = entity.pr_number
  end
  if not m_claims.verify_pr_review_issue_claim(core, "reconcile", repo, issue_number, nil, reconcile.proposal_id) then
    return
  end

  local lock_key = entity_lib.transition_lock_key(reconcile.proposal_id)
  if lock_key == nil then
    core.log_cas_decision("reconcile", reconcile.proposal_id, { state = nil, version = nil }, "reviewing", "blocked", "skip-foreign(proposal_id)", "no transition lock key")
    return
  end

  with_lock(lock_key, function()
    devloop_base.assert_trusted_bot_configured()

    local view = core.gh_pr_view_origin(repo, pr_number, 30)
    if view.exit_code ~= 0 then
      error("github-devloop: gh pr review reconcile view failed: " .. tostring(view.stderr))
    end

    local current = parsers_pr.parse_pr_view_origin(core, view.stdout)
    core.log_forged_markers("reconcile", reconcile.proposal_id, current.comments)
    local state = require("devloop.entity").current_entity_state(core, current.comments, reconcile.proposal_id)
    if conv_reconcile.has_review_reconcile_marker(core, current.comments, reconcile.proposal_id, reconcile.issue_version, reconcile.round) then
      core.log_cas_decision("reconcile", reconcile.proposal_id, state, "reviewing", "blocked", "skip-idempotent(review reconcile marker already visible)", "review reconcile result marker for incoming version is already visible")
      return
    end
    if state.state ~= nil and core.stage_rank(state.state) >= core.stage_rank("blocked") then
      core.log_cas_decision("reconcile", reconcile.proposal_id, state, "reviewing", "blocked", "skip-idempotent(already terminal)", "current marker is already terminal at or beyond blocked")
      return
    end
    if state.state == nil then
      core.log_cas_decision("reconcile", reconcile.proposal_id, state, "reviewing", "blocked", "pending", "reviewing state marker not yet visible")
      error("github-devloop: reviewing state marker not yet visible for review reconcile; retrying")
    end
    if state.state ~= "reviewing" then
      core.log_cas_decision("reconcile", reconcile.proposal_id, state, "reviewing", "blocked", "skip-stale(state-advanced)", "current marker advanced beyond reviewing")
      return
    end
    local version = conv_reconcile.review_reconcile_terminal_state_version(core, state.version, reconcile.round)
    local transition = core.versioned_transition_status(state, { "reviewing" }, "blocked", version)
    if transition == "pending" then
      core.log_cas_decision("reconcile", reconcile.proposal_id, state, "reviewing", "blocked", core.cas_outcome(state, transition, version), "reviewing state marker not yet visible")
      error("github-devloop: reviewing state marker not yet visible for review reconcile; retrying")
    end
    if transition == "idempotent" or transition == "stale" then
      core.log_cas_decision("reconcile", reconcile.proposal_id, state, "reviewing", "blocked", core.cas_outcome(state, transition, version), "current marker cannot be reconciled from reviewing")
      return
    end

    local action = "drop"
    local reason = "no-actionable-framing-after-" .. tostring(reconcile.round) .. "-review-rounds"
    local comment_request = core.build_review_reconcile_comment_request(repo, issue_number, reconcile, action, reason, version)
    local label_request = issue_number ~= nil and core.build_review_reconcile_label_request(repo, issue_number, reconcile) or nil
    emit_blocked_reconcile("reviewing", reconcile.proposal_id, state, version, action, reason, comment_request, label_request, "github-proxy.github_pr_comment_request")
  end)
end

local function pipeline_fix(event)
  local reconcile = event.payload or {}
  if not conv_reconcile.is_supported_fix_reconcile(core, reconcile) then
    core.log_entry("reconcile", event, "unknown", core.payload_field(reconcile, "dedup_key"))
    core.log_cas_decision("reconcile", "unknown", { state = nil, version = nil }, "reviewing", "blocked", "skip-foreign(proposal_id)", "unsupported event payload")
    return
  end

  core.log_entry("reconcile", event, reconcile.proposal_id, reconcile.dedup_key)
  local entity = entity_lib.parse_entity_proposal_id(reconcile.proposal_id)
  if entity == nil then
    core.log_cas_decision("reconcile", reconcile.proposal_id, { state = nil, version = nil }, "reviewing", "blocked", "skip-foreign(proposal_id)", "proposal_id is outside github-devloop")
    return
  end
  local repo = entity.repo
  local issue_number = entity.issue_number
  local _, pr_number = devloop_base.parse_pr_source_ref(reconcile.source_ref)
  if pr_number == nil then
    pr_number = entity.pr_number
  end
  if not m_claims.verify_pr_review_issue_claim(core, "reconcile", repo, issue_number, nil, reconcile.proposal_id) then
    return
  end

  local lock_key = entity_lib.transition_lock_key(reconcile.proposal_id)
  if lock_key == nil then
    core.log_cas_decision("reconcile", reconcile.proposal_id, { state = nil, version = nil }, "reviewing", "blocked", "skip-foreign(proposal_id)", "no transition lock key")
    return
  end

  with_lock(lock_key, function()
    devloop_base.assert_trusted_bot_configured()

    local view = core.gh_pr_view_origin(repo, pr_number, 30)
    if view.exit_code ~= 0 then
      error("github-devloop: gh pr fix reconcile view failed: " .. tostring(view.stderr))
    end

    local current = parsers_pr.parse_pr_view_origin(core, view.stdout)
    core.log_forged_markers("reconcile", reconcile.proposal_id, current.comments)
    local state = require("devloop.entity").current_entity_state(core, current.comments, reconcile.proposal_id)
    local version = conv_reconcile.fix_reconcile_state_version(core, reconcile.issue_version)
    if conv_reconcile.has_fix_reconcile_marker(core, current.comments, reconcile.proposal_id, reconcile.issue_version) then
      core.log_cas_decision("reconcile", reconcile.proposal_id, state, "reviewing", "blocked", "skip-idempotent(fix reconcile marker already visible)", "fix reconcile result marker for incoming version is already visible")
      return
    end
    if state.state ~= nil and core.stage_rank(state.state) >= core.stage_rank("blocked") then
      core.log_cas_decision("reconcile", reconcile.proposal_id, state, "reviewing", "blocked", "skip-idempotent(already terminal)", "current marker is already terminal at or beyond blocked")
      return
    end

    local transition = core.versioned_transition_status(state, { "reviewing" }, "blocked", version)
    if state.state == nil or transition == "pending" then
      core.log_cas_decision("reconcile", reconcile.proposal_id, state, "reviewing", "blocked", core.cas_outcome(state, transition, version), "reviewing state marker not yet visible")
      error("github-devloop: reviewing state marker not yet visible for fix reconcile; retrying")
    end
    if state.state ~= "reviewing"
      or transition_version.safe_version_segment(tostring(state.version or "")) ~= transition_version.safe_version_segment(tostring(reconcile.issue_version)) then
      core.log_cas_decision("reconcile", reconcile.proposal_id, state, "reviewing", "blocked", "skip-stale(version-mismatch)", "fix reconcile event does not match canonical reviewing marker")
      return
    end
    if transition == "idempotent" or transition == "stale" then
      core.log_cas_decision("reconcile", reconcile.proposal_id, state, "reviewing", "blocked", core.cas_outcome(state, transition, version), "current marker cannot be reconciled from reviewing")
      return
    end

    local action = "drop"
    local reason = "fix-loop-max-rounds-after-" .. tostring(reconcile.round) .. "-rounds"
    local comment_request = core.build_fix_reconcile_comment_request(repo, issue_number, reconcile, action, reason)
    local label_request = issue_number ~= nil and core.build_fix_reconcile_label_request(repo, issue_number, reconcile) or nil
    emit_blocked_reconcile("reviewing", reconcile.proposal_id, state, version, action, reason, comment_request, label_request, "github-proxy.github_pr_comment_request")
  end)
end

local function pipeline_timeout(event)
  local reconcile = event.payload or {}
  if not conv_reconcile.is_supported_timeout_reconcile(core, reconcile) then
    core.log_entry("reconcile", event, "unknown", core.payload_field(reconcile, "dedup_key"))
    core.log_cas_decision("reconcile", "unknown", { state = nil, version = nil }, "timeout", "blocked", "skip-foreign(proposal_id)", "unsupported event payload")
    return
  end

  core.log_entry("reconcile", event, reconcile.proposal_id, reconcile.dedup_key)
  local repo, issue_number = base_ids.parse_proposal_id(reconcile.proposal_id)
  local _, pr_number = devloop_base.parse_pr_source_ref(reconcile.source_ref)
  local lock_key = entity_lib.transition_lock_key(reconcile.proposal_id)
  if lock_key == nil then
    core.log_cas_decision("reconcile", reconcile.proposal_id, { state = nil, version = nil }, reconcile.state, "blocked", "skip-foreign(proposal_id)", "no transition lock key")
    return
  end

  with_lock(lock_key, function()
    devloop_base.assert_trusted_bot_configured()

    local comments
    local current_pr
    local current_issue
    local snapshot
    local target_pr_number = pr_number
    if pr_number ~= nil then
      if not m_claims.verify_pr_review_issue_claim(core, "reconcile", repo, issue_number, nil, reconcile.proposal_id) then
        return
      end
      local view = core.gh_pr_view_origin(repo, pr_number, 30)
      if view.exit_code ~= 0 then
        if not command_indicates_not_found(view) then
          error("github-devloop: gh pr timeout reconcile view failed: " .. tostring(view.stderr))
        end
        core.log_cas_decision("reconcile", reconcile.proposal_id, { state = reconcile.state, version = reconcile.issue_version }, reconcile.state, "blocked", "pr-surface-gone-fallback", "PR source disappeared before timeout reconcile; falling back to issue surface")
        target_pr_number = nil
        current_issue, current_pr, comments, snapshot = load_timeout_issue_surface(repo, issue_number, reconcile.proposal_id, reconcile.state)
      else
        current_pr = parsers_pr.parse_pr_view_origin(core, view.stdout)
        comments = current_pr.comments
      end
    else
      current_issue, current_pr, comments, snapshot = load_timeout_issue_surface(repo, issue_number, reconcile.proposal_id, reconcile.state)
    end

    core.log_forged_markers("reconcile", reconcile.proposal_id, comments)
    local state = require("devloop.entity").current_entity_state(core, comments, reconcile.proposal_id)
    if conv_reconcile.has_timeout_reconcile_marker(core, comments, reconcile.proposal_id, reconcile.issue_version, reconcile.state, reconcile.round) then
      core.log_cas_decision("reconcile", reconcile.proposal_id, state, reconcile.state, "blocked", "skip-idempotent(timeout reconcile marker already visible)", "timeout reconcile result marker for incoming version is already visible")
      return
    end
    local live_row = replay_fields.restart_transition_row(core.restart_transition_table(), state.state)
    if state.state ~= nil and live_row ~= nil and live_row.terminal == true then
      core.log_cas_decision("reconcile", reconcile.proposal_id, state, reconcile.state, "blocked", "skip-idempotent(already terminal)", "current marker is already terminal")
      return
    end
    if state.state == nil then
      core.log_cas_decision("reconcile", reconcile.proposal_id, state, reconcile.state, "blocked", "pending", "state marker not yet visible for timeout reconcile")
      error("github-devloop: state marker not yet visible for timeout reconcile; retrying")
    end
    if state.state ~= reconcile.state then
      core.log_cas_decision("reconcile", reconcile.proposal_id, state, reconcile.state, "blocked", "skip-stale(state-advanced)", "current marker advanced beyond timeout reconcile state")
      return
    end
    if transition_version.strip_suffixes(state.version) ~= transition_version.strip_suffixes(reconcile.issue_version) then
      core.log_cas_decision("reconcile", reconcile.proposal_id, state, reconcile.state, "blocked", "skip-stale(lineage-mismatch)", "timeout reconcile event does not match canonical state marker lineage")
      return
    end

    local row = replay_fields.restart_transition_row(core.restart_transition_table(), reconcile.state)
    local timeout_facts = {
      proposal_id = reconcile.proposal_id,
      current = { comments = comments },
      current_pr = current_pr,
      snapshot = snapshot,
      source_ref = reconcile.source_ref,
      head_sha = current_pr and current_pr.head_sha or nil,
      fresh_current_state = state,
    }
    if current_issue ~= nil then
      timeout_facts.current = current_issue
    end
    local epoch = row and row.actionable_epoch
    if type(epoch) == "table" and epoch.allows_state_entry_if_never_deferred == true then
      timeout_facts.dependency_gate = core.dependency_gate(repo, issue_number, {
        proposal_id = reconcile.proposal_id,
        version = state.version,
        comments = comments,
      })
    end
    local due, age_minutes = core.liveness_timeout_due_with_facts(row, state, timeout_facts, now())
    local decision = core.liveness_timeout_decision_with_facts(row, state, timeout_facts, now())
    if row
      and row.actionable_epoch
      and row.actionable_epoch.source == "live_defer_heartbeat:v1" then
      local signal = core.restart_row_liveness_signal(row, state, timeout_facts, now())
      age_minutes = signal.age_minutes or age_minutes
    end
    local limit = tonumber(row and row.on_timeout and row.on_timeout.escalate_after_attempts) or nil
    if not due or decision.action ~= "escalate" or tonumber(decision.attempt) < tonumber(reconcile.round) then
      core.log_cas_decision("reconcile", reconcile.proposal_id, state, reconcile.state, "blocked", "skip-stale(no-longer-over-budget)", "current marker is no longer at timeout escalation threshold")
      return
    end
    if reconcile.state == "blocked" then
      if conv_attempts.has_decompose_exhausted_marker(core, comments, reconcile.proposal_id, state.version) then
        core.log_cas_decision("reconcile", reconcile.proposal_id, state, "blocked", "devloop_decompose", "skip-idempotent(decompose-exhausted)", "blocked decompose output obligation already reached terminal stop")
        return
      end
      local target = target_pr_number ~= nil
        and { kind = "pr", repo = repo, number = target_pr_number }
        or { kind = "issue", repo = repo, number = issue_number }
      local comment_request = conv_attempts.build_decompose_exhausted_comment_request(core, target, reconcile.proposal_id, state, reconcile.source_ref, decision.attempt)
      local queue = target_pr_number ~= nil and "github-proxy.github_pr_comment_request" or "github-proxy.github_issue_comment_request"
      core.log_cas_decision("reconcile", reconcile.proposal_id, state, "blocked", "devloop_decompose", "applied(decompose-exhausted)", "blocked decompose output obligation exhausted")
      core.log_apply("reconcile", reconcile.proposal_id, nil, nil, { add = {}, remove = {} }, { queue })
      core.log_raise("reconcile", reconcile.proposal_id, queue, comment_request)
      return
    end

    local version = conv_reconcile.timeout_reconcile_state_version(core, state.version, reconcile.state, decision.attempt)
    local transition = core.versioned_transition_status(state, { reconcile.state }, "blocked", version)
    if transition == "pending" then
      core.log_cas_decision("reconcile", reconcile.proposal_id, state, reconcile.state, "blocked", core.cas_outcome(state, transition, version), "state marker not yet visible for timeout reconcile")
      error("github-devloop: state marker not yet visible for timeout reconcile; retrying")
    end
    if transition == "idempotent" or transition == "stale" then
      core.log_cas_decision("reconcile", reconcile.proposal_id, state, reconcile.state, "blocked", core.cas_outcome(state, transition, version), "current marker cannot be timeout reconciled")
      return
    end

    local action = "drop"
    local reason_prefix = row and row.on_timeout and row.on_timeout.on_escalate and row.on_timeout.on_escalate.reason
      or "state-output-obligation-timeout"
    local reason = tostring(reason_prefix) .. "-after-" .. tostring(decision.attempt) .. "-attempts"
    local reason_class = merge_wait_timeout_reason_class(reconcile, state, comments, current_pr)
    local why_fields = {
      from_state = reconcile.state,
      from_version = state.version,
      terminal_version = version,
      age_minutes = age_minutes,
      budget_minutes = row and row.budget and tonumber(row.budget.minutes) or nil,
      attempt = decision.attempt,
      attempt_limit = limit,
      driving_queue = row and row.driving_queue or nil,
      reason_class = reason_class,
      source_ref = base_ids.normalize_source_ref(reconcile.source_ref),
    }
    local comment_request = target_pr_number ~= nil
      and build_timeout_reconcile_pr_comment_request(repo, target_pr_number, reconcile, action, reason, version, why_fields)
      or conv_reconcile.build_timeout_reconcile_comment_request(core, repo, issue_number, reconcile, action, reason, version, why_fields)
    local label_request = requests_labels.build_state_label_request(core, repo, issue_number, "blocked", base_ids.dedup_key({
      "timeout-reconcile",
      "label",
      tostring(reconcile.dedup_key),
    }), reconcile.source_ref)
    emit_blocked_reconcile(
      reconcile.state,
      reconcile.proposal_id,
      state,
      version,
      action,
      reason,
      comment_request,
      label_request,
      target_pr_number ~= nil and "github-proxy.github_pr_comment_request" or nil
    )
  end)
end

return saga.department(spec, { done = function() return false end, act = function(event)
  local schema = core.payload_field(event and event.payload, "schema")
  if schema == "github-devloop.timeout-reconcile.v1" then
    return pipeline_timeout(event)
  end
  if schema == "github-devloop.review-reconcile.v1" then
    return pipeline_review(event)
  end
  if schema == "github-devloop.fix-reconcile.v1" then
    return pipeline_fix(event)
  end
end, wrap = core.wrap_pipeline_failure, name = "reconcile" })
