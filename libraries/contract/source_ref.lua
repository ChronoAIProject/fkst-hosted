-- contract.source_ref: structural helpers for stable {kind, ref} source pointers.
local strings = require("contract.strings")

local R = {}

function R.has_bounded_source_ref(source_ref, limit)
  return type(source_ref) == "table"
    and strings.is_bounded_string(source_ref.kind, limit)
    and strings.is_bounded_string(source_ref.ref, limit)
end

function R.version_order_key(version)
  local text = tostring(version or "")
  local rest = text
  if rest:sub(1, #"consensus:") == "consensus:" then
    rest = rest:sub(#"consensus:" + 1)
  elseif rest:sub(1, #"ready/") == "ready/" then
    rest = rest:sub(#"ready/" + 1):gsub("^consensus%-", "")
  end

  local timestamp = nil
  for found in rest:gmatch("(%d%d%d%d%-%d%d%-%d%dT%d%d[%-:]%d%d[%-:]%d%dZ)") do
    timestamp = found
  end
  if timestamp ~= nil then
    local _, end_pos = rest:find(timestamp, 1, true)
    local suffix = end_pos and rest:sub(end_pos + 1) or ""
    local loop_n = tonumber(suffix:match("/loop/(%d+)$")) or 0
    local suffix_tie = suffix:gsub("/loop/%d+$", "")
    return timestamp:gsub(":", "-") .. "/loop/" .. string.format("%012d", loop_n) .. suffix_tie
  end
  return rest
end

return R
