local core = require("core")
local saga = require("workflow.saga")

local LIVENESS_SCAN_CURSOR_PREFIX = "github-devloop/liveness-scan/issue-cursor/"

local spec = {
  consumes = { "devloop_liveness_tick" }, published_seam = { "devloop_liveness_tick" },
  produces = {
    "devloop_observe_issue",
    "github-proxy.github_issue_comment_request",
    "github-proxy.github_pr_comment_request",
    "consensus.proposal",
    "devloop_ready",
    "github-devloop-decompose.devloop_decompose",
    "devloop_reconcile",
    "devloop_timeout_reconcile",
  },
  fanout = { "devloop_liveness_tick" },
  stall_window = "30s",
}

local function should_reinject_issue(repo, issue, limits, deadline)
  if not core.issue_ref_round_trips(repo, issue.number) then
    return false
  end

  if not core.sweep_has_budget(deadline) then
    return nil, "deadline"
  end
  local proposal_id = core.proposal_id(repo, issue.number)
  local state_view = core.fetch_issue_view_state(repo, issue.number, issue.updated_at, {
    consumer = "liveness_scan",
    timeout = core.sweep_call_timeout(limits, deadline),
  })
  if state_view.exit_code ~= 0 then
    if core.liveness_scan_is_timeout_result(state_view) then
      return nil, "deadline"
    end
    error("github-devloop: liveness-scan-issue-view-failed: " .. tostring(state_view.stderr))
  end

  local current = core.parse_issue_view_state(state_view.stdout)
  if tostring(current.state or ""):upper() ~= "OPEN" then
    core.log_cas_decision("liveness_scan", proposal_id, { state = nil, version = nil }, "tick", "observe", "skip-closed", "issue is not open")
    return false
  end

  local state = core.current_entity_state(current.comments, proposal_id)
  if not core.liveness_scan_should_reinject_state(proposal_id, state) then
    return false
  end
  local snapshot = { comments = current.comments or {}, prs = {}, absent_prs = {}, state = state }
  local delegation = state.state == "awaiting-pr" and core.pr_delegation_fact(current.comments, proposal_id, state.version) or nil
  local current_pr = nil
  if delegation ~= nil then
    local pr_view = core.fetch_pr_view_origin(repo, delegation.pr_number, nil, {
      force_fresh = true,
      consumer = "liveness_scan",
    })
    if pr_view.exit_code ~= 0 then
      if core.liveness_scan_is_timeout_result(pr_view) then
        return nil, "deadline"
      end
      error("github-devloop: liveness-scan-awaiting-pr-view-failed: " .. tostring(pr_view.stderr))
    end
    current_pr = core.parse_pr_view_origin(pr_view.stdout)
    current_pr.number = delegation.pr_number
    snapshot.comments = current.comments or {}
  end
  local timeout_action = core.liveness_scan_maybe_timeout_action(core.liveness_scan_issue_entity(repo, issue.number), state, {
    proposal_id = proposal_id,
    current = { comments = current.comments or {}, labels = current.labels or {} },
    current_issue = current,
    current_pr = current_pr,
    ["pr-delegation"] = delegation,
    pr_delegation = delegation,
    snapshot = snapshot,
    event_ts = issue.updated_at,
    source_ref = core.issue_source_ref(repo, issue.number),
    fresh_current_state = state,
    now_seconds = now(),
  })
  if timeout_action == "handled" then
    return false
  end
  return true
end

local function liveness_scan_done(_event)
  return false
end

local function act_liveness_scan(event)
  core.log_entry("liveness_scan", event, "github-devloop/liveness-scan", "tick")
  core.assert_trusted_bot_configured()

  local repo = core.liveness_scan_read_repo()
  if repo == nil then
    core.log_cas_decision("liveness_scan", "github-devloop/liveness-scan", { state = nil, version = nil }, "tick", "observe", "skip-invalid-repo", "FKST_GITHUB_REPO is missing or invalid")
    return
  end

  local limits = core.liveness_scan_limits()
  local deadline = core.sweep_deadline(now(), limits)
  local timeout = core.sweep_call_timeout(limits, deadline)
  if timeout <= 0 then
    core.liveness_scan_log_deferred("deadline", { entity_cap = limits.entity_cap })
    return
  end
  local issues = core.liveness_scan_list_open_issues(repo, timeout, core.entity_list_poll_key(event))
  local activations, deferred_by_cap, cursor_key, cursor, total = core.liveness_scan_activation_slice(repo, "issue", issues, LIVENESS_SCAN_CURSOR_PREFIX)
  local processed = 0
  local attempted = 0

  for _, activation in ipairs(activations) do
    if not core.sweep_has_budget(deadline) then
      core.liveness_scan_update_cursor(cursor_key, cursor, total, attempted)
      core.liveness_scan_log_deferred("deadline", {
        listed_issues = #issues,
        processed = processed,
        deferred = (#activations - processed) + deferred_by_cap,
        entity_cap = limits.entity_cap,
      })
      return
    end

    attempted = attempted + 1
    local should_reinject, defer_reason = should_reinject_issue(repo, activation.entity, limits, deadline)
    if defer_reason == "deadline" then
      core.liveness_scan_update_cursor(cursor_key, cursor, total, attempted)
      core.liveness_scan_log_deferred("deadline", {
        listed_issues = #issues,
        processed = processed,
        deferred = (#activations - processed) + deferred_by_cap,
        entity_cap = limits.entity_cap,
      })
      return
    end
    processed = processed + 1
    if should_reinject then
      core.liveness_scan_reinject(repo, activation.entity, "issue", event and event.ts)
    end
  end

  core.liveness_scan_update_cursor(cursor_key, cursor, total, attempted)

  if deferred_by_cap > 0 then
    core.liveness_scan_log_deferred("cap", {
      listed_issues = #issues,
      processed = processed,
      deferred = deferred_by_cap,
      entity_cap = limits.entity_cap,
    })
  end
end

return saga.department(spec, {
  done = liveness_scan_done,
  act = act_liveness_scan,
  name = "liveness_scan",
})
