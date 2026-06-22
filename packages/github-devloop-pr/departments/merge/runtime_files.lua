local M = {}
local strings = require("contract.strings")

local MAX_RUNTIME_ID_LEN = 180

local function runtime_identity(repo, issue_number)
  local id = "merge-" .. strings.runtime_safe_segment(repo) .. "-issue-" .. strings.runtime_safe_segment(issue_number)
  if #id > MAX_RUNTIME_ID_LEN then
    return id:sub(1, MAX_RUNTIME_ID_LEN)
  end
  return id
end

function M.temp_body_file(repo, issue_number)
  return "/tmp/fkst-github-devloop-" .. runtime_identity(repo, issue_number) .. ".md"
end

return M
