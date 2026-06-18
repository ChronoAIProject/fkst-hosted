local core = require("core")
local saga = require("std.saga")

local spec = {
  consumes = { "devloop_intake_probe_tick" },
  produces = { "devloop_intake_candidate" },
  fanout = { "devloop_intake_probe_tick" },
  stall_window = "30s",
}

local PROBE_LIMIT = 5
local CURSOR_KEY = "github-devloop/intake-probe/created-cursor"
local BACKSTOP_BOUND = "burst beyond 5 new issues per 60s probe falls back to the 5m intake_scan sweep"

local function parse_cursor(value)
  local created_at, number = tostring(value or ""):match("^([^\t]+)\t(%d+)$")
  if created_at == nil then
    if value ~= nil and tostring(value) ~= "" then
      return tostring(value), 0
    end
    return nil, nil
  end
  return created_at, tonumber(number) or 0
end

local function encode_cursor(created_at, number)
  if created_at == nil or tostring(created_at) == "" then
    return nil
  end
  return tostring(created_at) .. "\t" .. tostring(tonumber(number) or 0)
end

local function api_since_from_cursor(created_at)
  local seconds = core.iso_timestamp_epoch_seconds(created_at)
  if seconds == nil then
    return created_at
  end
  return os.date("!%Y-%m-%dT%H:%M:%SZ", math.max(0, seconds - 1))
end

local function is_after_cursor(issue, cursor_created_at, cursor_number)
  if cursor_created_at == nil or cursor_created_at == "" then
    return true
  end
  local created_at = tostring(issue.created_at or "")
  local number = tonumber(issue.number) or 0
  return created_at ~= "" and (
    created_at > tostring(cursor_created_at)
      or (created_at == tostring(cursor_created_at) and number > (tonumber(cursor_number) or 0))
  )
end

local function maybe_raise_candidate(repo, issue, delivery_version)
  local issue_number = tostring(issue.number or "")
  if not core.issue_ref_round_trips(repo, issue_number) then
    return
  end
  local proposal_id = core.proposal_id(repo, issue_number)
  if core.should_skip_known_intake_issue(issue.labels) then
    core.log_cas_decision("intake_probe", proposal_id, { state = nil, version = nil }, "probe", "candidate", "skip-known-state", "probe list labels show an active devloop state")
    return
  end
  local payload = core.build_intake_scan_candidate(repo, issue, nil, delivery_version)
  core.log_apply("intake_probe", proposal_id, nil, nil, { add = {}, remove = {} }, {
    "devloop_intake_candidate",
  })
  core.log_raise("intake_probe", proposal_id, "devloop_intake_candidate", payload)
end

local function intake_probe_done(_event)
  return false
end

local function act_intake_probe(event)
  core.log_entry("intake_probe", event, "github-devloop/intake-probe", "tick")
  core.assert_trusted_bot_configured()

  local gate = core.intake_probe_gate()
  if not gate.enabled then
    core.log_cas_decision("intake_probe", "github-devloop/intake-probe", { state = nil, version = nil }, "tick", "candidate", "skip-gated", gate.reason .. "; " .. BACKSTOP_BOUND)
    return
  end

  local repo = core.read_intake_repo()
  if repo == nil then
    core.log_cas_decision("intake_probe", "github-devloop/intake-probe", { state = nil, version = nil }, "tick", "candidate", "skip-invalid-repo", "FKST_GITHUB_REPO is missing or invalid")
    return
  end

  local cursor_created_at, cursor_number = parse_cursor(cache_get(CURSOR_KEY))
  local listed = core.gh_issue_list_intake_probe(repo, PROBE_LIMIT, api_since_from_cursor(cursor_created_at), 30)
  if listed.exit_code ~= 0 then
    error("github-devloop: intake-probe-list-failed: " .. tostring(listed.stderr))
  end

  local issues = core.parse_issue_list_intake(listed.stdout, PROBE_LIMIT)
  local newest_created_at = nil
  local newest_number = nil
  for index, issue in ipairs(issues) do
    if index == 1 and issue.created_at ~= nil then
      newest_created_at = tostring(issue.created_at)
      newest_number = tonumber(issue.number) or 0
    end
    if is_after_cursor(issue, cursor_created_at, cursor_number) then
      maybe_raise_candidate(repo, issue, event.ts or now())
    end
  end
  local cursor = encode_cursor(newest_created_at, newest_number)
  if cursor ~= nil then
    cache_set(CURSOR_KEY, cursor)
  end
end

return saga.department(spec, {
  done = intake_probe_done,
  act = act_intake_probe,
  wrap = core.wrap_pipeline_failure,
  name = "intake_probe",
})
