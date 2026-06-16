local M = {}
local strings = require("std.strings")

local max_branch_len = 160

function M.url_encode(value)
  return (tostring(value or ""):gsub("([^%w%-%._~])", function(char)
    return string.format("%%%02X", string.byte(char))
  end))
end

function M.is_git_ref_safe(value)
  if not strings.is_bounded_string(value, max_branch_len) then
    return false
  end
  local text = tostring(value)
  if text:sub(1, 1) == "-" or text:sub(1, 1) == "/" then
    return false
  end
  if text:find("%.%.", 1, true) ~= nil
    or text:find("//", 1, true) ~= nil
    or text:find("@{", 1, true) ~= nil
    or text:sub(-1) == "/"
    or text:sub(-1) == "."
    or text:sub(-5) == ".lock" then
    return false
  end
  if text:find("[%s~^:?%[%]\\*]") ~= nil then
    return false
  end
  for segment in text:gmatch("[^/]+") do
    if segment == "." or segment == ".." or segment:sub(1, 1) == "." then
      return false
    end
  end
  return text:find("^[%w%._%-%/]+$") ~= nil
end

return M
