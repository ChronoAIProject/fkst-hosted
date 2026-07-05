local M = {}

local error_facts = require("contract.error_facts")

local github_query_timeout_seconds = 30

local function dense_list(value)
  if type(value) ~= "table" then
    return false
  end
  local count = 0
  local max_index = 0
  for key, _item in pairs(value) do
    if type(key) ~= "number" or key < 1 or math.floor(key) ~= key then
      return false
    end
    count = count + 1
    if key > max_index then
      max_index = key
    end
  end
  return max_index == count
end

local function parse_assigned_issue_count(stdout)
  if type(stdout) ~= "string" then
    return nil, "missing self-assigned issue query stdout"
  end
  local ok, decoded = pcall(json.decode, stdout)
  if not ok or not dense_list(decoded) then
    return nil, "malformed self-assigned issue query: response must be a JSON list"
  end
  for _, row in ipairs(decoded) do
    if type(row) ~= "table" then
      return nil, "malformed self-assigned issue query: issue row must be an object"
    end
    if type(row.number) ~= "number" or row.number < 1 or math.floor(row.number) ~= row.number then
      return nil, "malformed self-assigned issue query: issue number must be a positive integer"
    end
  end
  return #decoded, nil
end

function M.self_assigned_open_issue_verdict(github, identity)
  if type(github) ~= "table" or type(github.issue_list_open_assigned) ~= "function" then
    return {
      ok = false,
      error_class = "idle-assignee-query-unavailable",
      why = "self-assigned issue query failed: GitHub issue_list_open_assigned port is required",
    }
  end
  if type(identity) ~= "table" then
    return {
      ok = false,
      error_class = "github-claim-identity-unverified",
      why = "self-assigned issue query failed: claim identity is required",
    }
  end

  local ok, result = pcall(
    github.issue_list_open_assigned,
    identity.repo,
    identity.bot_login,
    github_query_timeout_seconds
  )
  if not ok then
    return {
      ok = false,
      error_class = "idle-assignee-query-failed",
      why = "self-assigned issue query failed: " .. error_facts.one_line(result),
      source_ref = identity.source_ref,
    }
  end
  if type(result) ~= "table" or result.exit_code ~= 0 then
    return {
      ok = false,
      error_class = "idle-assignee-query-failed",
      why = "self-assigned issue query failed: " .. error_facts.one_line(result and result.stderr or "no command result"),
      source_ref = identity.source_ref,
    }
  end

  local count, parse_err = parse_assigned_issue_count(result.stdout)
  if count == nil then
    return {
      ok = false,
      error_class = "idle-assignee-query-malformed",
      why = parse_err,
      source_ref = identity.source_ref,
    }
  end
  return {
    ok = true,
    count = count,
    source_ref = identity.source_ref,
  }
end

function M.build_system_idle_payload(detected_at, source_ref, expires_at)
  local payload = {
    schema = "idle-detector.system-idle.v1",
    detected_at = tostring(detected_at),
    source_ref = {
      kind = tostring(source_ref and source_ref.kind or "github-assignee-query"),
      ref = tostring(source_ref and source_ref.ref or ""),
    },
  }
  if expires_at ~= nil then
    payload.expires_at = tostring(expires_at)
  end
  return payload
end

function M.iso_timestamp_epoch_seconds(timestamp)
  local year, month, day, hour, minute, second = tostring(timestamp or ""):match(
    "^(%d%d%d%d)%-(%d%d)%-(%d%d)T(%d%d):(%d%d):(%d%d)Z$"
  )
  if year == nil then
    return nil
  end
  year, month, day = tonumber(year), tonumber(month), tonumber(day)
  hour, minute, second = tonumber(hour), tonumber(minute), tonumber(second)
  if month < 1 or month > 12 or day < 1 or day > 31 or hour > 23 or minute > 59 or second > 59 then
    return nil
  end
  if month <= 2 then
    year = year - 1
    month = month + 12
  end
  local era = math.floor(year / 400)
  local year_of_era = year - era * 400
  local day_of_year = math.floor((153 * (month - 3) + 2) / 5) + day - 1
  local day_of_era = year_of_era * 365 + math.floor(year_of_era / 4) - math.floor(year_of_era / 100) + day_of_year
  return (era * 146097 + day_of_era - 719468) * 86400 + hour * 3600 + minute * 60 + second
end

function M.freshness_verdict(reference_ts_seconds, now_seconds, budget_seconds)
  if type(reference_ts_seconds) ~= "number" or type(now_seconds) ~= "number" or type(budget_seconds) ~= "number" then
    error("idle-detector: malformed-freshness: timestamp inputs must be numeric")
  end
  if now_seconds - reference_ts_seconds > budget_seconds then
    return "stale"
  end
  return "fresh"
end

function M.skip_fact(dept, event, why, terminal)
  local fields = error_facts.error_fact_fields("terminal-skip", type(event) == "table" and event.queue or nil, dept, why, {
    source_ref = error_facts.event_source_ref(event),
    terminal = terminal,
  })
  table.insert(fields, "WHY=" .. error_facts.one_line(why))
  return "idle-detector dept=" .. tostring(dept) .. " tag=SKIP " .. table.concat(fields, " ")
end

return M
