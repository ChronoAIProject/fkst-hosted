local materialize_reconcile = require("materialize_reconcile")
local saga = require("workflow.saga")

local spec = {
  consumes = { "workflow_materialization_tick" },
  published_seam = { "workflow_materialization_tick" },
  produces = {
    "github-proxy.github_issue_create_request",
    "github-proxy.github_issue_comment_request",
  },
  stall_window = "2m",
}

return saga.department(spec, materialize_reconcile.handlers())
