local error_facts = require("contract.error_facts")
local ports_lib = require("forge.ports")
local saga = require("workflow.saga")

local spec = {
  consumes = { "health_tick" },
  produces = { "health_report_written" },
  -- Broadcast: any number of sibling packages may subscribe to "a health report was
  -- written", exactly as idle-detector broadcasts system_idle. Today nothing does,
  -- and the engine warns-and-drops an unsubscribed fanout queue -- the same shape
  -- github-comment-effect's github_comment_written already ships with.
  fanout = { "health_report_written" },
  stall_window = "10m",
  -- A missed window is not worth replaying: the next tick is ten minutes away and a
  -- stale report is worse than an absent one.
  retry = false,
}

-- Accept BOTH the bare and the namespaced production queue name. The engine
-- delivers "fkst-health.health_tick"; comparing event.queue against the bare literal
-- alone is what the G-NAMESPACED-QUEUE ratchet exists to catch.
local function is_health_tick(event)
  local queue = tostring(event and event.queue or "")
  return queue == "health_tick" or queue == "fkst-health.health_tick"
end

-- Failure posture (see the package README rationale in fkst.toml): this department
-- rides every session, so a defect is fleet-wide. Anything it cannot do degrades to
-- a log line; only an unroutable queue is a real error.
local function wrap_pipeline_failure(dept, fn)
  return function(event)
    local ok, result = pcall(fn, event)
    if ok then
      return result
    end
    local fields = error_facts.error_fact_fields(
      "caught-failure",
      type(event) == "table" and event.queue or nil,
      dept,
      result,
      { source_ref = error_facts.event_source_ref(event) }
    )
    table.insert(fields, "error=" .. error_facts.one_line(result))
    log["error"]("fkst-health dept=" .. dept .. " tag=FAILURE " .. table.concat(fields, " "))
    error(("fkst-health: caught-failure: " .. tostring(result)), 0)
  end
end

local function health_done(event)
  if not is_health_tick(event) then
    error("fkst-health: unknown-queue: " .. tostring(event and event.queue), 0)
  end
  return false
end

local function make_department(ports)
  ports = ports or {}

  local function act_health(event)
    if not is_health_tick(event) then
      error("fkst-health: unknown-queue: " .. tostring(event and event.queue), 0)
    end
    -- Scaffold only: the evidence collection, judgment codex, and report write land
    -- in the next change. The log line is the department's whole observable effect
    -- for now, and it is required -- the namespaced-dispatch conformance counts a
    -- route as live only when it produces activity, and file writes do not count.
    log.info("fkst-health dept=health_report tag=TICK queue=" .. tostring(event.queue))
  end

  local department = saga.department(spec, {
    done = health_done,
    act = act_health,
    wrap = wrap_pipeline_failure,
    name = "health_report",
  })
  department.ports = ports
  return department
end

return ports_lib.install(make_department)
