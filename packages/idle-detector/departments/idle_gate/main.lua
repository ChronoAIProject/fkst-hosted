local core = require("core")
local error_facts = require("contract.error_facts")
local saga = require("workflow.saga")

local spec = {
  consumes = { "idle_tick" },
  produces = { "system_idle" },
  -- system_idle is a broadcast signal: any number of sibling/host packages may
  -- subscribe to "system is idle" (archaudit audits, the website board
  -- re-renders, ...). Declare it fanout so multiple consumers each receive it.
  fanout = { "system_idle" },
  stall_window = "30s",
  retry = false,
}

local stale_budget_seconds = 10 * 60

local function iso_from_seconds(seconds)
  return os.date("!%Y-%m-%dT%H:%M:%SZ", tonumber(seconds))
end

local function tick_slot(event)
  local payload = type(event) == "table" and event.payload or {}
  return payload.slot or payload.cron_slot or payload.detected_at or (type(event) == "table" and event.ts)
end

local function log_skip(reason, event)
  log.warn(core.skip_fact("idle_gate", event, reason, true))
end

local function is_unresolved_observe_config_error(err)
  local message = tostring(err)
  return message:find("observe%-durable%-root%-unresolved", 1, false) ~= nil
end

local function wrap_pipeline_failure(dept, fn)
  return function(event)
    local ok, result = pcall(fn, event)
    if ok then
      return result
    end
    local fields = error_facts.error_fact_fields("caught-failure", type(event) == "table" and event.queue or nil, dept, result, {
      source_ref = error_facts.event_source_ref(event),
    })
    table.insert(fields, "error=" .. error_facts.one_line(result))
    log["error"]("idle-detector dept=" .. dept .. " tag=FAILURE " .. table.concat(fields, " "))
    error(("idle-detector: caught-failure: " .. tostring(result)), 0)
  end
end

local function slot_is_stale(slot, now_seconds)
  local slot_seconds = core.iso_timestamp_epoch_seconds(slot)
  if slot_seconds == nil then
    return true, "malformed or missing idle_tick slot"
  end
  if core.freshness_verdict(slot_seconds, now_seconds, stale_budget_seconds) == "stale" then
    return true, "stale idle_tick slot"
  end
  return false, nil
end

local function idle_done(_event)
  return false
end

local function act_idle(event)
  local slot = tick_slot(event)
  local ok_observe, facts_or_err = pcall(core.observe)
  if not ok_observe and is_unresolved_observe_config_error(facts_or_err) then
    error(tostring(facts_or_err), 0)
  end
  local observe_error = not ok_observe and ("unreadable observe facts: " .. tostring(facts_or_err)) or nil
  if observe_error ~= nil then return log_skip(observe_error, event) end
  local observe_now = core.observe_now_seconds(facts_or_err)
  local stale, stale_why = slot_is_stale(slot, observe_now)
  if stale then
    log_skip(stale_why, event)
    return
  end
  local idle, why = core.is_idle_observe(facts_or_err)
  if not idle then
    log_skip(why or "system busy", event)
    return
  end
  raise("system_idle", core.build_system_idle_payload(
    slot,
    "idle_tick/" .. tostring(slot),
    iso_from_seconds(core.iso_timestamp_epoch_seconds(slot) + stale_budget_seconds)
  ))
end

local M = saga.department(spec, {
  done = idle_done,
  act = act_idle,
  wrap = wrap_pipeline_failure,
  name = "idle_gate",
})
M.pipeline = _G.pipeline
return M
