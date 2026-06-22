local S = {}

S.valid_round = require("devloop.rounds").valid_round
S.strings = require("std.strings")
S.max_attr_len = 240

function S.marker_attr(marker, name)
  return marker:match(name .. '="([^"]*)"')
end

function S.safe_marker_attr(M, value, limit)
  local text = tostring(value or "")
  text = text:gsub("<!%-%- fkst:[^\n]*%-%->", " ")
  text = text:gsub("&lt;!%-%- fkst:[^\n]*%-%-&gt;", " ")
  text = text:gsub("%c", " "):gsub('"', "'"):gsub("[<>]", ""):gsub("%s+", " ")
  text = text:gsub("^%s+", ""):gsub("%s+$", "")
  local cap = limit or S.max_attr_len
  if #text > cap then
    text = M.truncate_utf8(text, cap)
  end
  return text
end

function S.decode_marker_attr(value)
  if type(value) ~= "string" or value == "" then
    return nil
  end
  if value:find("%c") ~= nil or value:find("[<>]") ~= nil or value:find('"', 1, true) ~= nil then
    return nil
  end
  return value
end

return S
