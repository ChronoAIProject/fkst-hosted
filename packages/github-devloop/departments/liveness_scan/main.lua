local core = require("core")

local M = {}
local LIVENESS_SCAN_MAX_PER_TICK = 100

M.spec = {
  consumes = { "devloop_liveness_tick" },
  produces = { "github-proxy.github_entity_changed" },
  fanout = { "devloop_liveness_tick" },
  stall_window = "30s",
}

local function read_repo()
  local repo = core.read_env("FKST_GITHUB_REPO")
  if repo == nil or not core.issue_ref_round_trips(repo, 1) then
    return nil
  end
  return repo
end

local function state_is_non_terminal(state)
  local row = core.restart_transition_row(state and state.state)
  return row ~= nil and row.terminal ~= true
end

local function build_observe_payload(repo, entity, kind, tick)
  local number = tostring(entity.number or "")
  local updated_at = tostring(entity.updated_at or "")
  local source_ref = kind == "pr" and core.pr_source_ref(repo, number) or core.issue_source_ref(repo, number)
  return {
    schema = "github-proxy.v1",
    type = kind,
    repo = repo,
    number = tonumber(number),
    state = entity.state,
    updated_at = updated_at,
    dedup_key = core._dedup_key({
      "liveness-scan",
      tostring(repo),
      kind,
      number,
      updated_at,
      tostring(tick or ""),
    }),
    source = "liveness-scan",
    source_ref = source_ref,
  }
end

local function should_reinject_state(proposal_id, state)
  if state == nil or state.state == nil then
    core.log_cas_decision("liveness_scan", proposal_id, { state = nil, version = nil }, "tick", "observe", "skip-no-state", "no current restart state marker")
    return false
  end
  if not state_is_non_terminal(state) then
    core.log_cas_decision("liveness_scan", proposal_id, state, "tick", "observe", "skip-terminal", "current restart state is terminal or unknown")
    return false
  end
  return true
end

local function should_reinject_issue(repo, issue)
  if not core.issue_ref_round_trips(repo, issue.number) then
    return false
  end

  local proposal_id = core.proposal_id(repo, issue.number)
  local state_view = core.fetch_issue_view_state(repo, issue.number, issue.updated_at, {
    consumer = "liveness_scan",
    fresh = true,
  })
  if state_view.exit_code ~= 0 then
    error("github-devloop: liveness-scan-issue-view-failed: " .. tostring(state_view.stderr))
  end

  local current = core.parse_issue_view_state(state_view.stdout)
  if tostring(current.state or ""):upper() ~= "OPEN" then
    core.log_cas_decision("liveness_scan", proposal_id, { state = nil, version = nil }, "tick", "observe", "skip-closed", "issue is not open")
    return false
  end

  return should_reinject_state(proposal_id, core.current_entity_state(current.comments, proposal_id))
end

local function should_reinject_pr(repo, pr)
  if not core._is_positive_pr_number(pr.number) then
    return false
  end

  local state_view = core.fetch_pr_view_origin(repo, pr.number, pr.updated_at, {
    consumer = "liveness_scan",
    fresh = true,
  })
  if state_view.exit_code ~= 0 then
    error("github-devloop: liveness-scan-pr-view-failed: " .. tostring(state_view.stderr))
  end

  local current = core.parse_pr_view_origin(state_view.stdout)
  local origin = core.pr_origin_fact(current.comments)
  local proposal_id = origin and origin.proposal_id or core.pr_proposal_id(repo, pr.number)
  if tostring(current.state or ""):upper() ~= "OPEN" then
    core.log_cas_decision("liveness_scan", proposal_id, { state = nil, version = nil }, "tick", "observe", "skip-closed", "PR is not open")
    return false
  end
  if origin == nil then
    core.log_cas_decision("liveness_scan", proposal_id, { state = nil, version = nil }, "tick", "observe", "skip-no-state", "PR has no origin marker")
    return false
  end

  return should_reinject_state(proposal_id, core.current_entity_state(current.comments, origin.proposal_id))
end

local function list_open_issues(repo)
  local list = core.gh_exec({ cmd = core.gh_issue_list_observe_cmd(repo), timeout = 60 })
  if list.exit_code ~= 0 then
    error("github-devloop: liveness-scan-issue-list-failed: " .. tostring(list.stderr))
  end
  return core.parse_issue_list_observe(list.stdout)
end

local function list_open_prs(repo)
  local list = core.gh_exec({ cmd = core.gh_pr_list_observe_cmd(repo), timeout = 60 })
  if list.exit_code ~= 0 then
    error("github-devloop: liveness-scan-pr-list-failed: " .. tostring(list.stderr))
  end
  return core.parse_pr_list_observe(list.stdout)
end

local function sort_by_number(items)
  table.sort(items, function(left, right)
    return tonumber(left.number or 0) < tonumber(right.number or 0)
  end)
  return items
end

local function activation_slice(issues, prs)
  local activations = {}
  for _, issue in ipairs(sort_by_number(issues or {})) do
    table.insert(activations, { kind = "issue", entity = issue })
  end
  for _, pr in ipairs(sort_by_number(prs or {})) do
    table.insert(activations, { kind = "pr", entity = pr })
  end
  local total = #activations
  if total > LIVENESS_SCAN_MAX_PER_TICK then
    local bounded = {}
    for index = 1, LIVENESS_SCAN_MAX_PER_TICK do
      table.insert(bounded, activations[index])
    end
    core.log_cas_decision("liveness_scan", "github-devloop/liveness-scan", { state = nil, version = nil }, "tick", "observe", "deferred-cap", tostring(total - LIVENESS_SCAN_MAX_PER_TICK) .. " open entities deferred by LIVENESS_SCAN_MAX_PER_TICK")
    return bounded
  end
  return activations
end

local function reinject(repo, entity, kind, tick)
  local proposal_id = kind == "pr" and core.pr_proposal_id(repo, entity.number) or core.proposal_id(repo, entity.number)
  local payload = build_observe_payload(repo, entity, kind, tick)
  core.log_apply("liveness_scan", proposal_id, nil, nil, { add = {}, remove = {} }, {
    "github-proxy.github_entity_changed",
  })
  core.log_raise("liveness_scan", proposal_id, "github-proxy.github_entity_changed", payload)
end

function pipeline(event)
  core.log_entry("liveness_scan", event, "github-devloop/liveness-scan", "tick")
  core.assert_trusted_bot_configured()

  local repo = read_repo()
  if repo == nil then
    core.log_cas_decision("liveness_scan", "github-devloop/liveness-scan", { state = nil, version = nil }, "tick", "observe", "skip-invalid-repo", "FKST_GITHUB_REPO is missing or invalid")
    return
  end

  for _, activation in ipairs(activation_slice(list_open_issues(repo), list_open_prs(repo))) do
    if activation.kind == "issue" and should_reinject_issue(repo, activation.entity) then
      reinject(repo, activation.entity, "issue", event and event.ts)
    elseif activation.kind == "pr" and should_reinject_pr(repo, activation.entity) then
      reinject(repo, activation.entity, "pr", event and event.ts)
    end
  end
end

return M
