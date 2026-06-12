local core = require("core")

local M = {}

M.spec = {
  consumes = { "devloop_thinking_reconcile_tick" },
  produces = { "github-proxy.github_entity_changed" },
  fanout = { "devloop_thinking_reconcile_tick" },
  stall_window = "30s",
}

local function read_repo()
  local repo = core.read_env("FKST_GITHUB_REPO")
  if repo == nil or not core.issue_ref_round_trips(repo, 1) then
    return nil
  end
  return repo
end

local function build_observe_payload(repo, issue, tick)
  local issue_number = tostring(issue.number or "")
  local updated_at = tostring(issue.updated_at or "")
  return {
    schema = "github-proxy.v1",
    type = "issue",
    repo = repo,
    number = tonumber(issue_number),
    state = issue.state,
    updated_at = updated_at,
    dedup_key = core._dedup_key({
      "thinking-reconcile",
      tostring(repo),
      "issue",
      issue_number,
      updated_at,
      tostring(tick or ""),
    }),
    source = "thinking-reconcile",
    source_ref = core.issue_source_ref(repo, issue_number),
  }
end

local function has_current_thinking_state(repo, issue)
  if not core.issue_ref_round_trips(repo, issue.number) then
    return false
  end

  local proposal_id = core.proposal_id(repo, issue.number)
  local state_view = core.fetch_issue_view_state(repo, issue.number, issue.updated_at, {
    consumer = "thinking_reconcile",
    fresh = true,
  })
  if state_view.exit_code ~= 0 then
    error("github-devloop: gh thinking reconcile issue view failed: " .. tostring(state_view.stderr))
  end

  local current = core.parse_issue_view_state(state_view.stdout)
  if tostring(current.state or ""):upper() ~= "OPEN" then
    core.log_cas_decision("thinking_reconcile", proposal_id, { state = nil, version = nil }, "tick", "observe", "skip-closed", "issue is not open")
    return false
  end

  local state = core.current_entity_state(current.comments, proposal_id)
  if state.state ~= "thinking" then
    core.log_cas_decision("thinking_reconcile", proposal_id, state, "tick", "observe", "skip-non-thinking", "thinking reconcile only rechecks current thinking state")
    return false
  end
  return true
end

function pipeline(event)
  core.log_entry("thinking_reconcile", event, "github-devloop/thinking-reconcile", "tick")
  core.assert_trusted_bot_configured()

  local repo = read_repo()
  if repo == nil then
    core.log_cas_decision("thinking_reconcile", "github-devloop/thinking-reconcile", { state = nil, version = nil }, "tick", "observe", "skip-invalid-repo", "FKST_GITHUB_REPO is missing or invalid")
    return
  end

  local list = core.gh_exec({ cmd = core.gh_issue_list_observe_cmd(repo, core._enabled_label), timeout = 60 })
  if list.exit_code ~= 0 then
    error("github-devloop: gh thinking reconcile issue list failed: " .. tostring(list.stderr))
  end

  for _, issue in ipairs(core.parse_issue_list_observe(list.stdout)) do
    if has_current_thinking_state(repo, issue) then
      local proposal_id = core.proposal_id(repo, issue.number)
      local payload = build_observe_payload(repo, issue, event and event.ts)
      core.log_apply("thinking_reconcile", proposal_id, nil, nil, { add = {}, remove = {} }, {
        "github-proxy.github_entity_changed",
      })
      core.log_raise("thinking_reconcile", proposal_id, "github-proxy.github_entity_changed", payload)
    end
  end
end

return M
