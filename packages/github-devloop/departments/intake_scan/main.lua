local core = require("core")

local M = {}

M.spec = {
  consumes = { "devloop_intake_tick" },
  produces = { "devloop_intake_candidate" },
  fanout = { "devloop_intake_tick" },
  stall_window = "30s",
}

local INTAKE_LIMIT = 100

local function has_devloop_state_label(labels)
  for _, label in ipairs(labels or {}) do
    if core._state_labels[tostring(label)] then
      return true
    end
  end
  return false
end

local function should_skip_known(labels)
  return core.is_opted_in(labels) or has_devloop_state_label(labels)
end

local function read_repo()
  local repo = core.devloop_config().repo
  if repo == nil or not core.issue_ref_round_trips(repo, 1) then
    return nil
  end
  return repo
end

function pipeline(event)
  core.log_entry("intake_scan", event, "github-devloop/intake", "tick")
  core.assert_trusted_bot_configured()

  local repo = read_repo()
  if repo == nil then
    core.log_cas_decision("intake_scan", "github-devloop/intake", { state = nil, version = nil }, "tick", "candidate", "skip-invalid-repo", "FKST_GITHUB_REPO is missing or invalid")
    return
  end

  local list = core.gh_exec({ cmd = core.gh_issue_list_intake_cmd(repo, INTAKE_LIMIT), timeout = 30 })
  if list.exit_code ~= 0 then
    error("github-devloop: gh issue intake list failed: " .. tostring(list.stderr))
  end

  for _, issue in ipairs(core.parse_issue_list_intake(list.stdout, INTAKE_LIMIT)) do
    local issue_number = tostring(issue.number or "")
    if core.issue_ref_round_trips(repo, issue_number) and not should_skip_known(issue.labels) then
      local proposal_id = core.proposal_id(repo, issue_number)
      local view = core.gh_exec({ cmd = core.gh_issue_view_intake_scan_cmd(repo, issue_number), timeout = 30 })
      if view.exit_code ~= 0 then
        error("github-devloop: gh issue intake scan view failed: " .. tostring(view.stderr))
      end
      local current = core.parse_issue_view_intake_scan(view.stdout)
      core.log_forged_markers("intake_scan", proposal_id, current.comments)
      if current.state == "OPEN"
        and not should_skip_known(current.labels)
        and not core.has_intake_decision_marker(current.comments, proposal_id) then
        local payload = core.build_devloop_intake_candidate_payload(repo, issue_number, issue.updated_at)
        core.log_apply("intake_scan", proposal_id, nil, nil, { add = {}, remove = {} }, {
          "devloop_intake_candidate",
        })
        core.log_raise("intake_scan", proposal_id, "devloop_intake_candidate", payload)
      end
    end
  end
end

return M
