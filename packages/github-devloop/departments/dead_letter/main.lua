local M = {}
local error_facts = require("std.error_facts")

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

  log.warn(
    "github-devloop dept=dead_letter tag=DEAD_LETTER"
      .. " delivery_id=" .. error_facts.one_line(payload.delivery_id)
      .. " queue=" .. error_facts.one_line(payload.queue)
      .. " dead_dept=" .. error_facts.one_line(payload.dept)
      .. " source_ref=" .. dead_source_ref(payload)
      .. " dedup_key=" .. error_facts.one_line(dead_dedup_key(payload))
      .. " attempt=" .. error_facts.one_line(payload.attempt)
      .. " error=" .. error_facts.one_line(payload.error)
  )
end

return M
