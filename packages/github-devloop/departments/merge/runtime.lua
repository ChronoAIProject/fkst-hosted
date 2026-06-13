local M = {}

local MAX_RUNTIME_ID_LEN = 180

local function safe_segment(value)
  local safe = tostring(value or ""):gsub("[^%w._-]", "_")
  safe = safe:gsub("_+", "_"):gsub("^_+", ""):gsub("_+$", "")
  if safe == "" then
    return "empty"
  end
  return safe
end

local function runtime_identity(repo, issue_number)
  local id = "merge-" .. safe_segment(repo) .. "-issue-" .. safe_segment(issue_number)
  if #id > MAX_RUNTIME_ID_LEN then
    return id:sub(1, MAX_RUNTIME_ID_LEN)
  end
  return id
end

function M.temp_body_file(repo, issue_number)
  return "/tmp/fkst-github-devloop-" .. runtime_identity(repo, issue_number) .. ".md"
end

return M
