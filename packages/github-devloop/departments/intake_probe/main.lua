local core = require("core")

local M = {}

M.spec = {
  consumes = { "devloop_intake_probe_tick" },
  produces = { "devloop_intake_candidate" },
  fanout = { "devloop_intake_probe_tick" },
  stall_window = "30s",
}

local PROBE_LIMIT = 5
local CURSOR_KEY = "github-devloop/intake-probe/created-at-cursor"

local function is_after_cursor(issue, cursor)
  if cursor == nil or cursor == "" then
    return true
  end
  local created_at = tostring(issue.created_at or "")
  return created_at ~= "" and created_at > tostring(cursor)
end

local function should_advance_cursor(issues, newest_created_at)
  return newest_created_at ~= nil and #issues < PROBE_LIMIT
end

local function maybe_raise_candidate(repo, issue)
  local issue_number = tostring(issue.number or "")
  if not core.issue_ref_round_trips(repo, issue_number) then
    return
  end
  local proposal_id = core.proposal_id(repo, issue_number)
  local view = core.gh_exec({ cmd = core.gh_issue_view_intake_scan_cmd(repo, issue_number), timeout = 30 })
  if view.exit_code ~= 0 then
    error("github-devloop: intake-probe-view-failed: " .. tostring(view.stderr))
  end
  local current = core.parse_issue_view_intake_scan(view.stdout)
  core.log_forged_markers("intake_probe", proposal_id, current.comments)
  if current.state == "OPEN"
    and not core.should_skip_known_intake_issue(current.labels)
    and not core.has_intake_decision_marker(current.comments, proposal_id)
    and core.claim_issue_for_management("intake_probe", repo, issue_number, current, proposal_id) then
    local payload = core.build_intake_scan_candidate(repo, issue, nil)
    core.log_apply("intake_probe", proposal_id, nil, nil, { add = {}, remove = {} }, {
      "devloop_intake_candidate",
    })
    core.log_raise("intake_probe", proposal_id, "devloop_intake_candidate", payload)
  end
end

function pipeline(event)
  core.log_entry("intake_probe", event, "github-devloop/intake-probe", "tick")
  core.assert_trusted_bot_configured()

  local repo = core.read_intake_repo()
  if repo == nil then
    core.log_cas_decision("intake_probe", "github-devloop/intake-probe", { state = nil, version = nil }, "tick", "candidate", "skip-invalid-repo", "FKST_GITHUB_REPO is missing or invalid")
    return
  end

  local listed = core.gh_exec({ cmd = core.gh_issue_list_intake_probe_cmd(repo, PROBE_LIMIT), timeout = 30 })
  if listed.exit_code ~= 0 then
    error("github-devloop: intake-probe-list-failed: " .. tostring(listed.stderr))
  end

  local issues = core.parse_issue_list_intake(listed.stdout, PROBE_LIMIT)
  local cursor = cache_get(CURSOR_KEY)
  local newest_created_at = nil
  for index, issue in ipairs(issues) do
    if index == 1 and issue.created_at ~= nil then
      newest_created_at = tostring(issue.created_at)
    end
    if is_after_cursor(issue, cursor) then
      maybe_raise_candidate(repo, issue)
    end
  end
  if should_advance_cursor(issues, newest_created_at) then
    cache_set(CURSOR_KEY, newest_created_at)
  end
end

pipeline = core.wrap_pipeline_failure("intake_probe", pipeline)

return M
