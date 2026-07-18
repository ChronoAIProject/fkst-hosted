-- OWN intake seat: discovers fkst-security review issues and drives the pipeline.
-- Claims via the fkst-security label + its own queues -- NEVER the dev intake
-- candidate seam. The spec lives here; the handlers come from bindings.lua.
local saga = require("workflow.saga")
local bindings = require("bindings")

local spec = {
  consumes = { "workflow_security_tick" },
  produces = {
    "workflow_security_materialization_tick",
    "github-comment-effect.github_issue_comment_request",
  },
  stall_window = "2m",
}

return saga.department(spec, bindings.intake_handlers())
