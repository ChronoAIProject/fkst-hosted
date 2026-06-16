-- std.error_facts: dependency-free primitives for stable failure fingerprints.
local F = {}

function F.one_line(value)
  return tostring(value or ""):gsub("%s+", " ")
end

function F.normalized_message(value)
  local text = F.one_line(value):lower()
  text = text:gsub("%d%d%d%d%-%d%d%-%d%d[tT ]%d%d:%d%d:%d%d%.?%d*Z?", "<time>")
  text = text:gsub("%f[%x]%x%x%x%x%x%x[%x]+%f[^%x]", "<sha>")
  text = text:gsub("/tmp/[^%s]+", "<path>")
  text = text:gsub("/var/folders/[^%s]+", "<path>")
  text = text:gsub("%s+", " ")
  return text
end

F.normalized_error_message = F.normalized_message

function F.stable_hash(value)
  local hash = 5381
  for index = 1, #value do
    hash = (hash * 33 + value:byte(index)) % 2147483647
  end
  return "fp-" .. tostring(hash)
end

function F.source_ref_field(source_ref)
  if type(source_ref) == "table" then
    return F.one_line(source_ref.kind) .. ":" .. F.one_line(source_ref.ref)
  end
  if source_ref ~= nil then
    return F.one_line(source_ref)
  end
  return nil
end

function F.error_fingerprint(error_class, queue, dept, message)
  return F.stable_hash(table.concat({
    tostring(error_class or "unknown-error"),
    tostring(queue or ""),
    tostring(dept or ""),
    F.normalized_message(message),
  }, "|"))
end

return F
