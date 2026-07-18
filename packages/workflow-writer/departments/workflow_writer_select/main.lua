-- OWN intake seat: discovers fkst-workflow request issues and drives the authoring
-- flow. Claims via the fkst-workflow label + its own queues -- NEVER the dev intake
-- candidate seam. The spec lives here; the handlers come from bindings.lua.
local saga = require("workflow.saga")
local bindings = require("bindings")

local spec = {
  consumes = { "workflow_authoring_request", "workflow_writer_tick" },
  produces = {
    "workflow_writer_materialization_tick",
    "github-proxy.github_issue_comment_request",
  },
  stall_window = "2m",
}

return saga.department(spec, bindings.intake_handlers())
