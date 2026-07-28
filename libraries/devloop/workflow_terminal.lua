local strings = require("contract.strings")
local parsers_misc = require("devloop.parsers.misc")

local M = {}

M.MAX_ORIGIN_PROPOSAL_ID_BYTES = 200
M.MAX_TERMINAL_REASON_CODE_BYTES = 128
M.TERMINAL_STATES = {
  done = true,
  blocked = true,
  error = true,
}

local terminal_marker_pattern = "<!%-%- fkst:github%-devloop%-workflow:terminal:v1.-%-%->"

local function attr(marker, name)
  return marker:match(name .. '="([^"]*)"')
end

local function valid_attr(value, limit)
  return type(value) == "string"
    and value ~= ""
    and #value <= limit
    and value:find("%c") == nil
    and value:find('"', 1, true) == nil
    and value:find("[<>]") == nil
    and strings.is_path_safe_key(value, limit)
end

local function fact_from_marker(marker, expected_origin)
  local origin = attr(marker, "origin")
  local state = attr(marker, "state")
  local reason_code = attr(marker, "reason_code")
  if origin ~= expected_origin
    or not valid_attr(origin, M.MAX_ORIGIN_PROPOSAL_ID_BYTES)
    or M.TERMINAL_STATES[state] ~= true
    or not valid_attr(reason_code, M.MAX_TERMINAL_REASON_CODE_BYTES) then
    return nil
  end
  return {
    origin = origin,
    state = state,
    reason_code = reason_code,
  }
end

function M.parse_marker(comment_body, expected_origin)
  if type(comment_body) ~= "string"
    or not valid_attr(expected_origin, M.MAX_ORIGIN_PROPOSAL_ID_BYTES) then
    return nil
  end

  local latest_marker = nil
  for marker in comment_body:gmatch(terminal_marker_pattern) do
    if attr(marker, "origin") == expected_origin then
      latest_marker = marker
    end
  end
  if latest_marker == nil then
    return nil
  end
  return fact_from_marker(latest_marker, expected_origin)
end

function M.latest_trusted_fact(comments, expected_origin)
  local fact = nil
  for _, comment in ipairs(parsers_misc._trusted_marker_comments(comments or {})) do
    fact = M.parse_marker(parsers_misc._comment_body(comment), expected_origin) or fact
  end
  return fact
end

return M
