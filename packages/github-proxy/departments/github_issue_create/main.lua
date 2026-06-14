local core = require("core")

local M = {}

M.spec = {
  consumes = { "github_issue_create_request" },
  produces = { "github_issue_blocked_by_request" },
  stall_window = "30s",
}

function pipeline(event)
  core.write_issue_create_request(event.payload or {})
end

pipeline = core.wrap_pipeline_failure("github_issue_create", pipeline)

return M
