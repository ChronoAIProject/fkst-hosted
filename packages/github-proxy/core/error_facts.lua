local S = {}

function S.install(M)

local function one_line(value)
  return tostring(value or ""):gsub("%s+", " ")
end

local function normalized_message(value)
  local text = one_line(value):lower()
  text = text:gsub("%d%d%d%d%-%d%d%-%d%d[tT ]%d%d:%d%d:%d%d%.?%d*Z?", "<time>")
  text = text:gsub("%f[%x]%x%x%x%x%x%x[%x]+%f[^%x]", "<sha>")
  text = text:gsub("/tmp/[^%s]+", "<path>")
  text = text:gsub("/var/folders/[^%s]+", "<path>")
  text = text:gsub("%s+", " ")
  return text
end

local function stable_hash(value)
  local hash = 5381
  for index = 1, #value do
    hash = (hash * 33 + value:byte(index)) % 2147483647
  end
  return "fp-" .. tostring(hash)
end

local function source_ref_field(source_ref)
  if type(source_ref) == "table" then
    return one_line(source_ref.kind) .. ":" .. one_line(source_ref.ref)
  end
  if source_ref ~= nil then
    return one_line(source_ref)
  end
  return nil
end

function M.error_fingerprint(error_class, queue, dept, message)
  return stable_hash(table.concat({
    tostring(error_class or "unknown-error"),
    tostring(queue or ""),
    tostring(dept or ""),
    normalized_message(message),
  }, "|"))
end

function M.error_class_from_message(message)
  local text = tostring(message or "")
  local class = text:match("github%-proxy: [^:]+ failed: ([%w%-]+):")
    or text:match("github%-proxy: ([%w%-]+):")
  return class or "caught-failure"
end

function M.error_fact_fields(error_class, queue, dept, message, context)
  local fields = {
    "error_class=" .. one_line(error_class or "unknown-error"),
    "fingerprint=" .. M.error_fingerprint(error_class, queue, dept, message),
  }
  local source_ref = source_ref_field(context and context.source_ref)
  if source_ref ~= nil and source_ref ~= "" then
    table.insert(fields, "source_ref=" .. source_ref)
  end
  if context and context.attempt ~= nil then
    table.insert(fields, "attempt=" .. one_line(context.attempt))
  end
  if context and context.terminal ~= nil then
    table.insert(fields, "terminal=" .. tostring(context.terminal == true))
  end
  return fields
end

function M.log_error_fact(level, dept, tag, error_class, queue, message, context)
  local fields = M.error_fact_fields(error_class, queue, dept, message, context)
  table.insert(fields, "queue=" .. one_line(queue))
  table.insert(fields, "error=" .. one_line(message))
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
      terminal = false,
    })
    error(err, 0)
  end
end

end

return S
