local core = require("core")
local error_facts = require("std.error_facts")

local M = {}

M.spec = {
  consumes = { "dead_letter" },
  produces = {},
  stall_window = "2m",
}

local function dead_source_ref(payload)
  local source_ref = payload.source_ref
  if source_ref == nil and type(payload.payload) == "table" then
    source_ref = payload.payload.source_ref
  end
  if type(source_ref) == "table" then
    return error_facts.source_ref_field(source_ref)
  end
  return error_facts.one_line(source_ref)
end

local function dead_dedup_key(payload)
  if payload.dedup_key ~= nil then
    return payload.dedup_key
  end
  if type(payload.payload) == "table" then
    return payload.payload.dedup_key
  end
  return nil
end

function pipeline(event)
  local payload = event.payload or {}
  local error_class = error_facts.one_line(payload.error_class or "dead-letter")
  local error_message = payload.error or payload.message or error_class
  local fields = core.error_fact_fields(error_class, payload.queue, payload.dept, error_message, {
    source_ref = payload.source_ref or (type(payload.payload) == "table" and payload.payload.source_ref or nil),
    attempt = payload.attempt,
    terminal = true,
  })

  log.warn(
    "consensus dept=dead_letter tag=DEAD_LETTER"
      .. " " .. table.concat(fields, " ")
      .. " delivery_id=" .. error_facts.one_line(payload.delivery_id)
      .. " queue=" .. error_facts.one_line(payload.queue)
      .. " dead_dept=" .. error_facts.one_line(payload.dept)
      .. " source_ref=" .. dead_source_ref(payload)
      .. " dedup_key=" .. error_facts.one_line(dead_dedup_key(payload))
      .. " attempt=" .. error_facts.one_line(payload.attempt)
      .. " error=" .. error_facts.one_line(payload.error)
  )
end

pipeline = core.wrap_pipeline_failure("dead_letter", pipeline)

return M
