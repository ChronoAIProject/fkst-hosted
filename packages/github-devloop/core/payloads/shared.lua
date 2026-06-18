local S = {}
local github_handle = nil

function S.install(M)
local github_view = require("std.github_view")
local label_names = github_view.label_names
local function github()
  if github_handle ~= nil then
    return github_handle
  end
  if type(exec_argv) ~= "function" then
    error("github-devloop: GitHub adapter requires exec_argv")
  end
  github_handle = require("std.github").new(exec_argv)
  return github_handle
end

local function bounded_framing(M, framing)
  if framing == nil then
    return nil
  end
  local value = tostring(framing)
  if #value > M._max_framing_len then
    value = M.truncate_utf8(value, M._max_framing_len)
  end
  return value
end

local function bounded_control_text(M, value, limit)
  if value == nil then
    return nil
  end
  local text = tostring(value):gsub("%c", " "):gsub("%s+", " ")
  text = text:gsub("^%s+", ""):gsub("%s+$", "")
  if text == "" then
    return nil
  end
  local cap = limit or M._max_blocking_gap_len
  if #text > cap then
    text = M.truncate_utf8(text, cap)
  end
  return text
end

return {
  github = github,
  label_names = label_names,
  bounded_framing = bounded_framing,
  bounded_control_text = bounded_control_text,
}
end

return S
