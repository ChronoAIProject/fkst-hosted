local core = require("core")

local M = {}

M.spec = {
  consumes = { "devloop_reconcile", "devloop_review_reconcile", "devloop_fix_reconcile" },
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

local function pipeline_thinking(event)
  local reconcile = event.payload or {}
  if not core.is_supported_reconcile(reconcile) then
    core.log_entry("reconcile", event, "unknown", reconcile.dedup_key)
    core.log_cas_decision("reconcile", "unknown", { state = nil, version = nil }, "thinking", "blocked", "skip-foreign(proposal_id)", "unsupported event payload")
    return
  end

  core.log_entry("reconcile", event, reconcile.proposal_id, reconcile.dedup_key)
  local repo, issue_number = core.parse_proposal_id(reconcile.proposal_id)
  if repo == nil then
    core.log_cas_decision("reconcile", reconcile.proposal_id, { state = nil, version = nil }, "thinking", "blocked", "skip-foreign(proposal_id)", "proposal_id is outside github-devloop")
    return
  end

  local lock_key = core.loop_lock_key(reconcile.proposal_id)
  if lock_key == nil then
    core.log_cas_decision("reconcile", reconcile.proposal_id, { state = nil, version = nil }, "thinking", "blocked", "skip-foreign(proposal_id)", "no transition lock key")
    return
  end

  with_lock(lock_key, function()
    core.assert_trusted_bot_configured()

    local view = exec_sync({ cmd = core.gh_issue_view_loop_cmd(repo, issue_number), timeout = 30 })
    if view.exit_code ~= 0 then
      error("github-devloop: gh issue reconcile view failed: " .. tostring(view.stderr))
    end

    local current = core.parse_issue_view_loop(view.stdout)
    core.log_forged_markers("reconcile", reconcile.proposal_id, current.comments)
    local state = core.current_state(current.comments, reconcile.proposal_id)
    local version = core.reconcile_state_version(reconcile.base_version, reconcile.round)
    if core.has_reconcile_marker(current.comments, reconcile.proposal_id, reconcile.base_version, reconcile.round) then
      core.log_cas_decision("reconcile", reconcile.proposal_id, state, "thinking", "blocked", "skip-idempotent(reconcile marker already visible)", "reconcile result marker for incoming version is already visible")
      return
    end
    if state.state ~= nil and core.stage_rank(state.state) >= core.stage_rank("blocked") then
      core.log_cas_decision("reconcile", reconcile.proposal_id, state, "thinking", "blocked", "skip-idempotent(already terminal)", "current marker is already terminal at or beyond blocked")
      return
    end

    local transition = core.versioned_transition_status(state, { "thinking" }, "blocked", version)
    if state.state == nil or transition == "pending" then
      core.log_cas_decision("reconcile", reconcile.proposal_id, state, "thinking", "blocked", core.cas_outcome(state, transition, version), "thinking state marker not yet visible")
      error("github-devloop: thinking state marker not yet visible for reconcile; retrying")
    end
    if transition == "idempotent" or transition == "stale" then
      core.log_cas_decision("reconcile", reconcile.proposal_id, state, "thinking", "blocked", core.cas_outcome(state, transition, version), "current marker cannot be reconciled from thinking")
      return
    end

    -- re-design/re-cluster require a trusted directive fact; current deterministic reconcile drops.
    local action = "drop"
    local reason = "no-actionable-framing-after-" .. tostring(reconcile.round) .. "-rounds"
    local comment_request = core.build_reconcile_comment_request(repo, issue_number, reconcile, action, reason)
    local label_request = core.build_reconcile_label_request(repo, issue_number, reconcile)
    emit_blocked_reconcile("thinking", reconcile.proposal_id, state, version, action, reason, comment_request, label_request)
  end)
end

local function pipeline_review(event)
  local reconcile = event.payload or {}
  if not core.is_supported_review_reconcile(reconcile) then
    core.log_entry("reconcile", event, "unknown", reconcile.dedup_key)
    core.log_cas_decision("reconcile", "unknown", { state = nil, version = nil }, "reviewing", "blocked", "skip-foreign(proposal_id)", "unsupported event payload")
    return
  end

  core.log_entry("reconcile", event, reconcile.proposal_id, reconcile.dedup_key)
  local entity = core.parse_entity_proposal_id(reconcile.proposal_id)
  if entity == nil then
    core.log_cas_decision("reconcile", reconcile.proposal_id, { state = nil, version = nil }, "reviewing", "blocked", "skip-foreign(proposal_id)", "proposal_id is outside github-devloop")
    return
  end
  local repo = entity.repo
  local issue_number = entity.issue_number
  local _, pr_number = core.parse_pr_source_ref(reconcile.source_ref)
  if pr_number == nil then
    pr_number = entity.pr_number
  end

  local lock_key = core.transition_lock_key(reconcile.proposal_id)
  if lock_key == nil then
    core.log_cas_decision("reconcile", reconcile.proposal_id, { state = nil, version = nil }, "reviewing", "blocked", "skip-foreign(proposal_id)", "no transition lock key")
    return
  end

  with_lock(lock_key, function()
    core.assert_trusted_bot_configured()

    local view = exec_sync({ cmd = core.gh_pr_view_origin_cmd(repo, pr_number), timeout = 30 })
    if view.exit_code ~= 0 then
      error("github-devloop: gh pr review reconcile view failed: " .. tostring(view.stderr))
    end

    local current = core.parse_pr_view_origin(view.stdout)
    core.log_forged_markers("reconcile", reconcile.proposal_id, current.comments)
    local state = core.current_entity_state(current.comments, reconcile.proposal_id)
    local version = core.review_reconcile_state_version(reconcile.issue_version, reconcile.round)
    if core.has_review_reconcile_marker(current.comments, reconcile.proposal_id, reconcile.issue_version, reconcile.round) then
      core.log_cas_decision("reconcile", reconcile.proposal_id, state, "reviewing", "blocked", "skip-idempotent(review reconcile marker already visible)", "review reconcile result marker for incoming version is already visible")
      return
    end
    if state.state ~= nil and core.stage_rank(state.state) >= core.stage_rank("blocked") then
      core.log_cas_decision("reconcile", reconcile.proposal_id, state, "reviewing", "blocked", "skip-idempotent(already terminal)", "current marker is already terminal at or beyond blocked")
      return
    end

    local transition = core.versioned_transition_status(state, { "reviewing" }, "blocked", version)
    if state.state == nil or transition == "pending" then
      core.log_cas_decision("reconcile", reconcile.proposal_id, state, "reviewing", "blocked", core.cas_outcome(state, transition, version), "reviewing state marker not yet visible")
      error("github-devloop: reviewing state marker not yet visible for review reconcile; retrying")
    end
    if state.state ~= "reviewing"
      or core.safe_version_segment(tostring(state.version or "")) ~= core.safe_version_segment(tostring(reconcile.issue_version)) then
      core.log_cas_decision("reconcile", reconcile.proposal_id, state, "reviewing", "blocked", "skip-stale(version-mismatch)", "review reconcile event does not match canonical reviewing marker")
      return
    end
    if transition == "idempotent" or transition == "stale" then
      core.log_cas_decision("reconcile", reconcile.proposal_id, state, "reviewing", "blocked", core.cas_outcome(state, transition, version), "current marker cannot be reconciled from reviewing")
      return
    end

    local action = "drop"
    local reason = "no-actionable-framing-after-" .. tostring(reconcile.round) .. "-review-rounds"
    local comment_request = core.build_review_reconcile_comment_request(repo, issue_number, reconcile, action, reason)
    local label_request = issue_number ~= nil and core.build_review_reconcile_label_request(repo, issue_number, reconcile) or nil
    emit_blocked_reconcile("reviewing", reconcile.proposal_id, state, version, action, reason, comment_request, label_request, "github-proxy.github_pr_comment_request")
  end)
end

local function pipeline_fix(event)
  local reconcile = event.payload or {}
  if not core.is_supported_fix_reconcile(reconcile) then
    core.log_entry("reconcile", event, "unknown", reconcile.dedup_key)
    core.log_cas_decision("reconcile", "unknown", { state = nil, version = nil }, "reviewing", "blocked", "skip-foreign(proposal_id)", "unsupported event payload")
    return
  end

  core.log_entry("reconcile", event, reconcile.proposal_id, reconcile.dedup_key)
  local entity = core.parse_entity_proposal_id(reconcile.proposal_id)
  if entity == nil then
    core.log_cas_decision("reconcile", reconcile.proposal_id, { state = nil, version = nil }, "reviewing", "blocked", "skip-foreign(proposal_id)", "proposal_id is outside github-devloop")
    return
  end
  local repo = entity.repo
  local issue_number = entity.issue_number
  local _, pr_number = core.parse_pr_source_ref(reconcile.source_ref)
  if pr_number == nil then
    pr_number = entity.pr_number
  end

  local lock_key = core.transition_lock_key(reconcile.proposal_id)
  if lock_key == nil then
    core.log_cas_decision("reconcile", reconcile.proposal_id, { state = nil, version = nil }, "reviewing", "blocked", "skip-foreign(proposal_id)", "no transition lock key")
    return
  end

  with_lock(lock_key, function()
    core.assert_trusted_bot_configured()

    local view = exec_sync({ cmd = core.gh_pr_view_origin_cmd(repo, pr_number), timeout = 30 })
    if view.exit_code ~= 0 then
      error("github-devloop: gh pr fix reconcile view failed: " .. tostring(view.stderr))
    end

    local current = core.parse_pr_view_origin(view.stdout)
    core.log_forged_markers("reconcile", reconcile.proposal_id, current.comments)
    local state = core.current_entity_state(current.comments, reconcile.proposal_id)
    local version = core.fix_reconcile_state_version(reconcile.issue_version)
    if core.has_fix_reconcile_marker(current.comments, reconcile.proposal_id, reconcile.issue_version) then
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
      or core.safe_version_segment(tostring(state.version or "")) ~= core.safe_version_segment(tostring(reconcile.issue_version)) then
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

function pipeline(event)
  local payload = event.payload or {}
  if payload.schema == "github-devloop.review-reconcile.v1" then
    return pipeline_review(event)
  end
  if payload.schema == "github-devloop.fix-reconcile.v1" then
    return pipeline_fix(event)
  end
  return pipeline_thinking(event)
end

return M
