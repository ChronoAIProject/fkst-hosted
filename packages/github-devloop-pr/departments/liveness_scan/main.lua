local core, sweep_bounds = require("core"), require("devloop.sweep_bounds")
local saga = require("workflow.saga")

local LIVENESS_SCAN_CURSOR_PREFIX = "github-devloop-pr/liveness-scan/pr-cursor/"

local spec = {
  consumes = { "devloop_liveness_tick" },
  produces = {
    "devloop_observe_pr",
    "github-proxy.github_issue_comment_request",
    "github-proxy.github_pr_comment_request",
    "consensus.proposal",
    "devloop_reviewing",
    "devloop_fixing",
    "devloop_review_meta",
    "devloop_merge_ready",
    "devloop_review_reconcile",
    "devloop_timeout_reconcile",
  },
  fanout = { "devloop_liveness_tick" },
  stall_window = "30s",
}

local function should_reinject_pr(repo, pr, limits, deadline)
  if not core._is_positive_pr_number(pr.number) then
    return false
  end

  if not sweep_bounds.sweep_has_budget(deadline) then
    return nil, "deadline"
  end
  local state_view = core.fetch_pr_view_origin(repo, pr.number, pr.updated_at, {
    consumer = "liveness_scan",
    timeout = sweep_bounds.sweep_call_timeout(limits, deadline),
  })
  if state_view.exit_code ~= 0 then
    if core.liveness_scan_is_timeout_result(state_view) then
      return nil, "deadline"
    end
    error("github-devloop: liveness-scan-pr-view-failed: " .. tostring(state_view.stderr))
  end

  local current = core.parse_pr_view_origin(state_view.stdout)
  current.number = pr.number
  local origin = core.pr_origin_fact(current.comments)
  local proposal_id = origin and origin.proposal_id or core.pr_proposal_id(repo, pr.number)
  if origin == nil then
    core.log_cas_decision("liveness_scan", proposal_id, { state = nil, version = nil }, "tick", "observe", "skip-no-state", "PR has no origin marker")
    return false
  end
  if not core.verify_pr_review_issue_claim("liveness_scan", origin.repo, origin.issue_number, nil, origin.proposal_id) then
    return false
  end

  local state = core.current_entity_state(current.comments, origin.proposal_id)
  if not core.liveness_scan_should_reinject_state(proposal_id, state) then
    return false
  end
  local source_ref = core.pr_source_ref(repo, pr.number)
  local timeout_action = core.liveness_scan_maybe_timeout_action(core.liveness_scan_issue_entity(origin.repo, origin.issue_number), state, {
    proposal_id = origin.proposal_id,
    current = { comments = current.comments or {}, labels = current.labels or {} },
    current_pr = current,
    link = {
      proposal_id = origin.proposal_id,
      pr_number = pr.number,
      branch = origin.branch,
      impl_version = origin.impl_version,
      base_branch = origin.base_branch,
    },
    snapshot = {
      comments = current.comments or {},
      prs = { { number = pr.number, current = current } },
      state = state,
    },
    source_ref = source_ref,
    head_sha = current.head_sha,
    review_proposal_id = state.state == "reviewing" and core._is_git_sha(current.head_sha)
      and core.pr_review_proposal_id(origin.repo, pr.number, state.version, current.head_sha)
      or nil,
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
  local deadline = sweep_bounds.sweep_deadline(now(), limits)
  local timeout = sweep_bounds.sweep_call_timeout(limits, deadline)
  if timeout <= 0 then
    core.liveness_scan_log_deferred("deadline", { entity_cap = limits.entity_cap })
    return
  end
  local prs = core.liveness_scan_list_open_prs(repo, timeout, core.entity_list_poll_key(event))
  local activations, deferred_by_cap, cursor_key, cursor, total = core.liveness_scan_activation_slice(repo, "pr", prs, LIVENESS_SCAN_CURSOR_PREFIX)
  local processed = 0
  local attempted = 0

  for _, activation in ipairs(activations) do
    if not sweep_bounds.sweep_has_budget(deadline) then
      core.liveness_scan_update_cursor(cursor_key, cursor, total, attempted)
      core.liveness_scan_log_deferred("deadline", {
        listed_prs = #prs,
        processed = processed,
        deferred = (#activations - processed) + deferred_by_cap,
        entity_cap = limits.entity_cap,
      })
      return
    end

    attempted = attempted + 1
    local should_reinject, defer_reason = should_reinject_pr(repo, activation.entity, limits, deadline)
    if defer_reason == "deadline" then
      core.liveness_scan_update_cursor(cursor_key, cursor, total, attempted)
      core.liveness_scan_log_deferred("deadline", {
        listed_prs = #prs,
        processed = processed,
        deferred = (#activations - processed) + deferred_by_cap,
        entity_cap = limits.entity_cap,
      })
      return
    end
    processed = processed + 1
    if should_reinject then
      core.liveness_scan_reinject(repo, activation.entity, "pr", event and event.ts)
    end
  end

  core.liveness_scan_update_cursor(cursor_key, cursor, total, attempted)

  if deferred_by_cap > 0 then
    core.liveness_scan_log_deferred("cap", {
      listed_prs = #prs,
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
