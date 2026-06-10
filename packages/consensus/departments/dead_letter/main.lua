local M = {}

M.spec = {
  consumes = { "dead_letter" },
  produces = {},
  stall_window = "2m",
}

local function one_line(value)
  return tostring(value or ""):gsub("%s+", " ")
end

function pipeline(event)
  local payload = event.payload or {}

  log.warn(
    "consensus dept=dead_letter tag=DEAD_LETTER"
      .. " delivery_id=" .. one_line(payload.delivery_id)
      .. " queue=" .. one_line(payload.queue)
      .. " dead_dept=" .. one_line(payload.dept)
      .. " attempt=" .. one_line(payload.attempt)
      .. " error=" .. one_line(payload.error)
  )
end

return M
