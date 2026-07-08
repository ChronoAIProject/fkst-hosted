local strings = require("contract.strings")

local M = {}

M.MARKER_PREFIX = "[fkst:blocked-github-content:v1"
M.REDACTION_REASON = "non-whitelisted-author"

local known_array_fields = {
  assignees = true,
  comments = true,
  labels = true,
  nodes = true,
  reviews = true,
  statusCheckRollup = true,
}

local function is_array(value, key_hint)
  if type(value) ~= "table" then
    return false
  end
  local count = 0
  local saw_key = false
  for key, _ in pairs(value) do
    saw_key = true
    if type(key) ~= "number" or key < 1 or key % 1 ~= 0 then
      return false
    end
    if key > count then
      count = key
    end
  end
  for i = 1, count do
    if value[i] == nil then
      return false
    end
  end
  if not saw_key then
    return known_array_fields[tostring(key_hint or "")] == true
  end
  return true
end

local function json_value(value, key_hint)
  local value_type = type(value)
  if value == nil or value_type == "userdata" then
    return "null"
  end
  if value_type == "string" then
    return strings.json_string(value)
  end
  if value_type == "number" then
    return tostring(value)
  end
  if value_type == "boolean" then
    return value and "true" or "false"
  end
  if value_type == "table" then
    if is_array(value, key_hint) then
      local parts = {}
      for i = 1, #value do
        parts[#parts + 1] = json_value(value[i])
      end
      return "[" .. table.concat(parts, ",") .. "]"
    end
    local keys = {}
    for key, _ in pairs(value) do
      keys[#keys + 1] = tostring(key)
    end
    table.sort(keys)
    local parts = {}
    for _, key in ipairs(keys) do
      parts[#parts + 1] = strings.json_string(key) .. ":" .. json_value(value[key], key)
    end
    return "{" .. table.concat(parts, ",") .. "}"
  end
  error("github-devloop: content provenance cannot encode " .. value_type)
end

local function author_login(value)
  if type(value) ~= "table" then
    return nil
  end
  if type(value.author) == "table" then
    return value.author.login
  end
  if type(value.user) == "table" then
    return value.user.login
  end
  return nil
end

function M.canon_login(login)
  if login == nil then
    return nil
  end
  local value = strings.trim(login):lower():gsub("%[bot%]$", "")
  value = strings.trim(value)
  if value == "" then
    return nil
  end
  return value
end

function M.build_whitelist(logins)
  local set = {}
  for _, login in ipairs(logins or {}) do
    local canonical = M.canon_login(login)
    if canonical ~= nil then
      set[canonical] = true
    end
  end
  return set
end

function M.is_authorized(author_login_value, whitelist)
  local canonical = M.canon_login(author_login_value)
  if canonical == nil then
    return false
  end
  return type(whitelist) == "table" and whitelist[canonical] == true
end

function M.redaction_marker(field, author_login_value, bytes_removed)
  local login = M.canon_login(author_login_value) or "unknown"
  return M.MARKER_PREFIX
    .. ' field="' .. tostring(field) .. '"'
    .. ' existed="true"'
    .. ' author_login="' .. login .. '"'
    .. ' bytes_removed="' .. tostring(bytes_removed or 0) .. '"'
    .. ' why="' .. M.REDACTION_REASON .. '"]'
end

function M.filter_cell(body, author_login_value, field, whitelist)
  if M.is_authorized(author_login_value, whitelist) then
    return body, nil
  end
  local bytes_removed = (type(body) == "userdata" or body == nil) and 0 or #tostring(body)
  local marker = M.redaction_marker(field, author_login_value, bytes_removed)
  return marker, {
    field = field,
    author_login = M.canon_login(author_login_value),
    bytes_removed = bytes_removed,
    reason = M.REDACTION_REASON,
  }
end

local function filter_field(entity, json_field, redaction_field, author, whitelist, records)
  if type(entity) ~= "table" or entity[json_field] == nil then
    return
  end
  local filtered, record = M.filter_cell(entity[json_field], author, redaction_field, whitelist)
  entity[json_field] = filtered
  if record ~= nil and type(records) == "table" then
    records[#records + 1] = record
  end
end

function M.filter_gh_content_json(json_string, kind, whitelist, records)
  if kind ~= "issue" and kind ~= "pr" then
    error("github-devloop: content provenance invalid kind")
  end
  local ok, decoded = pcall(json.decode, json_string or "")
  if not ok or type(decoded) ~= "table" then
    error("github-devloop: content provenance JSON decode failed")
  end

  local found = {}
  if type(decoded.comments) == "table" then
    for _, comment in ipairs(decoded.comments) do
      if type(comment) == "table" then
        filter_field(comment, "body", "comment.body", author_login(comment), whitelist, found)
      end
    end
  end

  -- Preserve the original bytes when nothing was redacted, so unfiltered content
  -- (the common case) stays byte-identical to the fetched gh JSON and does not
  -- churn downstream byte-sensitive consumers.
  if #found == 0 then
    return json_string
  end
  if type(records) == "table" then
    for _, record in ipairs(found) do
      records[#records + 1] = record
    end
  end
  return json_value(decoded) .. "\n"
end

M._json_value = json_value

return M
