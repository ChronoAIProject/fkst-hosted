local M = {}

local error_facts = require("contract.error_facts")

local observe_schema_version = 1

local function required_list(facts, name)
  local value = facts[name]
  if type(value) ~= "table" then
    error("idle-detector: malformed-observe-facts: malformed " .. name)
  end
  local count = 0
  local max_index = 0
  for key, _item in pairs(value) do
    if type(key) ~= "number" or key < 1 or math.floor(key) ~= key then
      error("idle-detector: malformed-observe-facts: malformed " .. name)
    end
    count = count + 1
    if key > max_index then
      max_index = key
    end
  end
  if max_index ~= count then
    error("idle-detector: malformed-observe-facts: malformed " .. name)
  end
  return value
end

local function required_int(row, name)
  local value = row[name]
  if type(value) ~= "number" or value < 0 or math.floor(value) ~= value then
    error("idle-detector: malformed-observe-metric: " .. tostring(name) .. " must be a non-negative integer")
  end
  return value
end

local function required_table(facts, name)
  local value = facts[name]
  if type(value) ~= "table" then
    error("idle-detector: malformed-observe-facts: malformed " .. name)
  end
  return value
end

local function required_bool(row, name)
  local value = row[name]
  if type(value) ~= "boolean" then
    error("idle-detector: malformed-observe-facts: " .. tostring(name) .. " must be a boolean")
  end
  return value
end

local function validate_observe_facts(facts)
  if type(facts) ~= "table" then
    error("idle-detector: malformed-observe-facts: top-level facts must be a table")
  end
  if facts.schema_version ~= observe_schema_version then
    error("idle-detector: unknown-observe-schema-version: expected schema_version=1")
  end
  if type(facts.generated_at_ms) ~= "number" or facts.generated_at_ms < 0 or math.floor(facts.generated_at_ms) ~= facts.generated_at_ms then
    error("idle-detector: malformed-observe-facts: generated_at_ms must be a non-negative integer")
  end
  required_table(facts, "source")
  local limits = required_table(facts, "limits")
  required_int(limits, "max_deliveries")
  required_int(limits, "max_dead_letters")
  local truncated = required_table(facts, "truncated")
  required_bool(truncated, "deliveries")
  required_bool(truncated, "dead_letters")
  required_list(facts, "queues")
  required_list(facts, "deliveries")
  required_list(facts, "dead_letters")
  for _, row in ipairs(facts.queues) do
    if type(row) ~= "table" then
      error("idle-detector: malformed-observe-row: queue row must be a table")
    end
    if type(row.queue) ~= "string" or row.queue == "" then
      error("idle-detector: malformed-observe-row: queue name must be non-empty")
    end
    required_int(row, "depth")
    required_int(row, "pending")
    required_int(row, "in_flight")
    required_int(row, "retrying")
  end
  return facts
end

function M.observe_now_seconds(facts)
  validate_observe_facts(facts)
  return math.floor(facts.generated_at_ms / 1000)
end

function M.observe(opts)
  if type(fkst) ~= "table" or type(fkst.observe) ~= "function" then
    error("idle-detector: missing-observe: fkst.observe is required")
  end
  local ok, facts = pcall(fkst.observe, opts)
  if not ok then
    local message = tostring(facts)
    if message:find("FKST_DURABLE_ROOT", 1, true) ~= nil then
      error("idle-detector: observe-durable-root-unresolved: " .. message)
    end
    if message:find("fkst.observe snapshot", 1, true) ~= nil then
      error("idle-detector: malformed-observe-facts: " .. message)
    end
    error("idle-detector: observe-unreadable: " .. message)
  end
  return validate_observe_facts(facts)
end

function M.is_idle_observe(facts)
  validate_observe_facts(facts)
  if facts.truncated.deliveries then
    return false, "observe truncated deliveries"
  end
  if facts.truncated.dead_letters then
    return false, "observe truncated dead_letters"
  end
  for _, row in ipairs(facts.queues) do
    local queue = row.queue
    for _, field in ipairs({ "pending", "in_flight", "retrying", "depth" }) do
      if row[field] > 0 then
        return false, "busy queue=" .. queue .. " " .. field .. "=" .. tostring(row[field])
      end
    end
  end
  if #facts.deliveries > 0 then
    return false, "busy deliveries=" .. tostring(#facts.deliveries)
  end
  if #facts.dead_letters > 0 then
    return false, "busy dead_letters=" .. tostring(#facts.dead_letters)
  end
  return true, nil
end

function M.build_system_idle_payload(detected_at, observe_ref, expires_at)
  local payload = {
    schema = "idle-detector.system-idle.v1",
    detected_at = tostring(detected_at),
    source_ref = {
      kind = "host-observe",
      ref = tostring(observe_ref),
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
