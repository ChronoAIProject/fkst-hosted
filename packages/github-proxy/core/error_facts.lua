local S = {}

function S.install(M)
local error_facts = require("std.error_facts")

function M.error_fingerprint(error_class, queue, dept, message)
  return error_facts.error_fingerprint(error_class, queue, dept, message)
end

function M.error_class_from_message(message)
  local text = tostring(message or "")
  local class = text:match("github%-proxy: [^:]+ failed: ([%w%-]+):")
    or text:match("github%-proxy: ([%w%-]+):")
  return class or "caught-failure"
end

function M.error_fact_fields(error_class, queue, dept, message, context)
  local fields = {
    "error_class=" .. error_facts.one_line(error_class or "unknown-error"),
    "fingerprint=" .. M.error_fingerprint(error_class, queue, dept, message),
  }
  local source_ref = error_facts.source_ref_field(context and context.source_ref)
  if source_ref ~= nil and source_ref ~= "" then
    table.insert(fields, "source_ref=" .. source_ref)
  end
  if context and context.attempt ~= nil then
    table.insert(fields, "attempt=" .. error_facts.one_line(context.attempt))
  end
  if context and context.terminal ~= nil then
    table.insert(fields, "terminal=" .. tostring(context.terminal == true))
  end
  return fields
end

function M.log_error_fact(level, dept, tag, error_class, queue, message, context)
  local fields = M.error_fact_fields(error_class, queue, dept, message, context)
  table.insert(fields, "queue=" .. error_facts.one_line(queue))
  table.insert(fields, "error=" .. error_facts.one_line(message))
  M.log_line(level or "warn", dept, tag or "FAILURE", fields)
end

local function event_source_ref(event)
  if type(event) == "table" and event.source_ref ~= nil then
    return event.source_ref
  end
  local payload = type(event) == "table" and event.payload or nil
  if type(payload) == "table" then
    return payload.source_ref
  end
  return nil
end

function M.wrap_pipeline_failure(dept, fn)
  return function(event)
    local ok, err = pcall(fn, event)
    if ok then
      return err
    end
    M.log_error_fact("error", dept, "FAILURE", M.error_class_from_message(err), type(event) == "table" and event.queue or nil, err, {
      source_ref = event_source_ref(event),
      attempt = type(event) == "table" and event.attempt or nil,
    })
    error(err, 0)
  end
end

end

return S
