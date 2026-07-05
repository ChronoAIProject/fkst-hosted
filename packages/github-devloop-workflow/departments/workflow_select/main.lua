local saga = require("workflow.saga")
local workflow_select = require("workflow_select")

local spec = {
  consumes = { "github-devloop-intake.devloop_intake_candidate" },
  produces = {
    "github-devloop.devloop_execute_request",
    "github-proxy.github_issue_comment_request",
    "github-proxy.github_issue_create_request",
    "github-proxy.github_issue_label_request",
    "github-proxy.github_pr_comment_request",
  },
  stall_window = "2m",
}

return saga.department(spec, workflow_select.handlers())
