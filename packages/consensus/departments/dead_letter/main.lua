local M = {}

M.spec = {
  consumes = { "dead_letter" },
  produces = {},
  stall_window = "2m",
}

local function pick(...)
  for index = 1, select("#", ...) do
    local value = select(index, ...)
    if value ~= nil then
      return value
    end
  end
  return nil
end

local function field(payload, name)
  local delivery = type(payload.delivery) == "table" and payload.delivery or {}
  local event = type(payload.event) == "table" and payload.event or {}
  local original = type(payload.payload) == "table" and payload.payload or {}

  return pick(
    payload[name],
    delivery[name],
    event[name],
    original[name],
    type(event.payload) == "table" and event.payload[name] or nil,
    type(delivery.payload) == "table" and delivery.payload[name] or nil
  )
end

local function source_ref_text(source_ref)
  if type(source_ref) ~= "table" then
    return tostring(source_ref)
  end
  return tostring(source_ref.kind) .. ":" .. tostring(source_ref.ref)
end

function pipeline(event)
  local payload = event.payload or {}
  local queue = pick(field(payload, "queue"), field(payload, "source_queue"), event.queue)
  local dept = pick(field(payload, "dept"), field(payload, "department"))
  local source_ref = field(payload, "source_ref")
  local dedup_key = field(payload, "dedup_key")

  log.warn(
    "consensus dept=dead_letter tag=DEAD_LETTER"
      .. " queue=" .. tostring(queue)
      .. " dead_dept=" .. tostring(dept)
      .. " source_ref=" .. source_ref_text(source_ref)
      .. " dedup_key=" .. tostring(dedup_key)
  )
end

return M
