local core = require("core")
local env = require("workflow.env")
local error_facts = require("contract.error_facts")
local claim_identity = require("forge.github.claim_identity")
local ports_lib = require("forge.ports")
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

local allowed_env = {
  FKST_GITHUB_REPO = true,
  FKST_GITHUB_BOT_LOGIN = true,
}

local function read_env_command(name)
  if not allowed_env[name] then
    error("idle-detector: env-name-denied: " .. tostring(name), 0)
  end
  return 'printf %s "$' .. name .. '"'
end

local production_read_env = env.read_env(read_env_command, { propagate_exec_errors = true })

local function production_now()
  if type(now) ~= "function" then
    error("idle-detector: now-unavailable: now primitive is required", 0)
  end
  return now()
end

local function iso_from_seconds(seconds)
  return os.date("!%Y-%m-%dT%H:%M:%SZ", tonumber(seconds))
end

local function is_idle_tick(event)
  local queue = tostring(event and event.queue or "")
  return queue == "idle_tick" or queue == "idle-detector.idle_tick"
end

local function tick_slot(event)
  local payload = type(event) == "table" and event.payload or {}
  return payload.slot or payload.cron_slot or payload.detected_at or (type(event) == "table" and event.ts)
end

local function log_skip(reason, event)
  log.warn(core.skip_fact("idle_gate", event, reason, true))
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
    return true, "malformed or missing idle_tick slot", nil
  end
  if core.freshness_verdict(slot_seconds, now_seconds, stale_budget_seconds) == "stale" then
    return true, "stale idle_tick slot", slot_seconds
  end
  return false, nil, slot_seconds
end

local function idle_done(event)
  if not is_idle_tick(event) then
    error("idle-detector: unknown-queue: " .. tostring(event and event.queue), 0)
  end
  return false
end

local function make_department(ports)
  ports = ports or {}
  local github = ports.github
  local read_env = ports.read_env or production_read_env
  local now_seconds = ports.now or production_now

  local function act_idle(event)
    if not is_idle_tick(event) then
      error("idle-detector: unknown-queue: " .. tostring(event and event.queue), 0)
    end

    local slot = tick_slot(event)
    local stale, stale_why, slot_seconds = slot_is_stale(slot, tonumber(now_seconds()))
    if stale then
      log_skip(stale_why, event)
      return
    end

    local identity, identity_err = claim_identity.read(read_env)
    if identity_err ~= nil then
      log_skip(identity_err.why, event)
      return
    end

    local verdict = core.self_assigned_open_issue_verdict(github, identity)
    if not verdict.ok then
      log_skip(verdict.why or "self-assigned issue query failed", event)
      return
    end
    if verdict.count > 0 then
      log_skip("busy self_assigned_open_issues=" .. tostring(verdict.count), event)
      return
    end

    raise("system_idle", core.build_system_idle_payload(
      slot,
      verdict.source_ref,
      iso_from_seconds(slot_seconds + stale_budget_seconds)
    ))
  end

  local department = saga.department(spec, {
    done = idle_done,
    act = act_idle,
    wrap = wrap_pipeline_failure,
    name = "idle_gate",
  })
  department.ports = ports
  return department
end

return ports_lib.install(make_department)
