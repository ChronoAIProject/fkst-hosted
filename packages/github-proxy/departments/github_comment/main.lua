local issue_comment = require("github-proxy-effects.issue_comment_department")
local saga = require("workflow.saga")

local spec = {
  consumes = { "github_issue_comment_request" },
  published_seam = { "github_issue_comment_request" },
  produces = { "github_comment_written" },
  stall_window = "30s",
}

return saga.department(spec, {
  done = issue_comment.done,
  act = issue_comment.act,
  wrap = issue_comment.wrap,
  name = issue_comment.name,
})
