local core = require("core")

local M = {}

M.spec = {
  consumes = { "github_pr_comment_request" },
  stall_window = "30s",
}

function pipeline(event)
  local payload = event.payload or {}
  core.write_comment_request(payload, {
    kind = "pr",
    number = payload.pr_number,
    number_field = "pr_number",
    view_comments_cmd = core.gh_pr_view_comments_cmd,
    comment_cmd = core.gh_pr_comment_cmd,
    view_label = "gh pr view",
    comment_label = "gh pr comment",
  })
end

return M
