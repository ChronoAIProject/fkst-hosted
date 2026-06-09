local core = require("core")

local M = {}

M.spec = {
  consumes = { "github_issue_comment_request" },
  stall_window = "30s",
}

function pipeline(event)
  local payload = event.payload or {}
  core.write_comment_request(payload, {
    kind = "issue",
    number = payload.issue_number,
    number_field = "issue_number",
    view_comments_cmd = core.gh_issue_view_comments_cmd,
    comment_cmd = core.gh_issue_comment_cmd,
    view_label = "gh issue view",
    comment_label = "gh issue comment",
  })
end

return M
