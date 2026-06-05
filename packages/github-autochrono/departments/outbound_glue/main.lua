local core = require("core")

local M = {}

M.spec = {
  consumes = { "autochrono.reply" },
  produces = { "github-proxy.github_issue_comment_request" },
  stall_window = "30s",
}

function pipeline(event)
  raise("github-proxy.github_issue_comment_request", core.reply_to_comment_request(event.payload or {}))
end

return M
