local devloop_base = require("devloop.base")
local entity_lib = require("devloop.entity")
local m_claims = require("devloop.claims")
local parsers_pr = require("devloop.parsers.pr")
local m_facts = require("devloop.markers.facts")
local core, sweep_bounds = require("core"), require("devloop.sweep_bounds")
local liveness_scan = require("devloop.liveness_scan")
local entity_list_cache = require("devloop.entity_list_cache")
local saga = require("workflow.saga")
local forge_validators = require("devloop.forge_validators")
local devloop_entity_view = require("devloop.github_proxy_entity_view")

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
  if not forge_validators.is_positive_pr_number(pr.number) then
    return false
  end

  if not sweep_bounds.sweep_has_budget(deadline) then
    return nil, "deadline"
  end
  local state_view = devloop_entity_view.fetch_pr_view_origin(repo, pr.number, pr.updated_at, {
    consumer = "liveness_scan",
    timeout = sweep_bounds.sweep_call_timeout(limits, deadline),
  })
  if state_view.exit_code ~= 0 then
    if liveness_scan.liveness_scan_is_timeout_result(core, state_view) then
      return nil, "deadline"
    end
    error("github-devloop: liveness-scan-pr-view-failed: " .. tostring(state_view.stderr))
  end

  local current = parsers_pr.parse_pr_view_origin(core, state_view.stdout)
  current.number = pr.number
  local origin = m_facts.pr_origin_fact(core, current.comments)
  local proposal_id = origin and origin.proposal_id or entity_lib.pr_proposal_id(repo, pr.number)
  if origin == nil then
    core.log_cas_decision("liveness_scan", proposal_id, { state = nil, version = nil }, "tick", "observe", "skip-no-state", "PR has no origin marker")
    return false
  end
  if not m_claims.verify_pr_review_issue_claim(core, "liveness_scan", origin.repo, origin.issue_number, nil, origin.proposal_id) then
    return false
  end

  local state = require("devloop.entity").current_entity_state(core, current.comments, origin.proposal_id)
  if not liveness_scan.liveness_scan_should_reinject_state(core, proposal_id, state) then
    return false
  end
  local source_ref = entity_lib.pr_source_ref(repo, pr.number)
  local timeout_action = liveness_scan.liveness_scan_maybe_timeout_action(core, liveness_scan.liveness_scan_issue_entity(core, origin.repo, origin.issue_number), state, {
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
    review_proposal_id = state.state == "reviewing" and forge_validators.is_git_sha(current.head_sha)
      and devloop_base.pr_review_proposal_id(origin.repo, pr.number, state.version, current.head_sha)
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
  devloop_base.assert_trusted_bot_configured()

  local repo = liveness_scan.liveness_scan_read_repo(core)
  if repo == nil then
    core.log_cas_decision("liveness_scan", "github-devloop/liveness-scan", { state = nil, version = nil }, "tick", "observe", "skip-invalid-repo", "FKST_GITHUB_REPO is missing or invalid")
    return
  end

  local limits = liveness_scan.liveness_scan_limits(core)
  local deadline = sweep_bounds.sweep_deadline(now(), limits)
  local timeout = sweep_bounds.sweep_call_timeout(limits, deadline)
  if timeout <= 0 then
    liveness_scan.liveness_scan_log_deferred(core, "deadline", { entity_cap = limits.entity_cap })
    return
  end
  local prs = liveness_scan.liveness_scan_list_open_prs(core, repo, timeout, entity_list_cache.entity_list_poll_key(core, event))
  local activations, deferred_by_cap, cursor_key, cursor, total = liveness_scan.liveness_scan_activation_slice(core, repo, "pr", prs, LIVENESS_SCAN_CURSOR_PREFIX)
  local processed = 0
  local attempted = 0

  for _, activation in ipairs(activations) do
    if not sweep_bounds.sweep_has_budget(deadline) then
      liveness_scan.liveness_scan_update_cursor(core, cursor_key, cursor, total, attempted)
      liveness_scan.liveness_scan_log_deferred(core, "deadline", {
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
      liveness_scan.liveness_scan_update_cursor(core, cursor_key, cursor, total, attempted)
      liveness_scan.liveness_scan_log_deferred(core, "deadline", {
        listed_prs = #prs,
        processed = processed,
        deferred = (#activations - processed) + deferred_by_cap,
        entity_cap = limits.entity_cap,
      })
      return
    end
    processed = processed + 1
    if should_reinject then
      liveness_scan.liveness_scan_reinject(core, repo, activation.entity, "pr", event and event.ts)
    end
  end

  liveness_scan.liveness_scan_update_cursor(core, cursor_key, cursor, total, attempted)

  if deferred_by_cap > 0 then
    liveness_scan.liveness_scan_log_deferred(core, "cap", {
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
