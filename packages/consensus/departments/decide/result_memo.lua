local M = {}

local function key_less(left, right)
  local left_type = type(left)
  local right_type = type(right)
  if left_type ~= right_type then
    return left_type < right_type
  end
  if left_type == "number" then
    return left < right
  end
  return tostring(left) < tostring(right)
end

local function encode_value(value, seen)
  local value_type = type(value)
  if value_type == "boolean" then
    return tostring(value)
  end
  if value_type == "number" then
    if value ~= value or value == math.huge or value == -math.huge then
      error("consensus: result-memo-invalid: memo numbers must be finite")
    end
    return tostring(value)
  end
  if value_type == "string" then
    return string.format("%q", value)
  end
  if value_type ~= "table" then
    error("consensus: result-memo-invalid: unsupported memo value type " .. value_type)
  end

  seen = seen or {}
  if seen[value] then
    error("consensus: result-memo-invalid: recursive memo table")
  end
  seen[value] = true

  local keys = {}
  for key in pairs(value) do
    local key_type = type(key)
    if key_type ~= "number" and key_type ~= "string" then
      error("consensus: result-memo-invalid: unsupported memo key type " .. key_type)
    end
    table.insert(keys, key)
  end
  table.sort(keys, key_less)

  local parts = {}
  for _, key in ipairs(keys) do
    table.insert(parts, "[" .. encode_value(key, seen) .. "]=" .. encode_value(value[key], seen))
  end
  seen[value] = nil
  return "{" .. table.concat(parts, ",") .. "}"
end

local function plain_data(value)
  if type(value) ~= "table" then
    return value
  end
  local copied = {}
  for key, nested in pairs(value) do
    copied[plain_data(key)] = plain_data(nested)
  end
  return copied
end

function M.encode(payload)
  return encode_value(payload)
end

function M.load(key, expected_dedup_key)
  local encoded = cache_get(key)
  if encoded == nil then
    return nil
  end
  local ok, payload = pcall(restricted_lua_load, {
    source = "return " .. encoded,
    bindings = {},
    mode = "text",
    name = "consensus-result-memo",
  })
  if not ok or type(payload) ~= "table" then
    error("consensus: result-memo-invalid: memo payload cannot be decoded")
  end
  payload = plain_data(payload)
  if payload.schema ~= "consensus.consensus_reached.v1"
    or payload.dedup_key ~= "consensus:" .. tostring(expected_dedup_key) then
    error("consensus: result-memo-invalid: memo payload identity mismatch")
  end
  return payload
end

function M.save(key, payload)
  cache_set(key, M.encode(payload))
end

return M
