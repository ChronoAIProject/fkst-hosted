local core = require("core")

local M = {}

M.spec = {
  consumes = { "github_issue_create_request" },
  stall_window = "30s",
}

function pipeline(event)
  core.write_issue_create_request(event.payload or {})
end

return M
