local core, saga = require("core"), require("contract.saga")


local spec = {
  consumes = { "devloop_observe_tick" },
  produces = { "github-proxy.github_issue_create_request" },
  graph_json = true,
  retry = false,
  stall_window = "2m",
}

local department = saga.department(spec, { done = function() return false end, act = function(event)
  core.log_entry("observability", event, "github-devloop/observability", "tick")
  core.observe_devloop_entities(event)
end, wrap = core.wrap_pipeline_failure, name = "observability" })
department.spec.graph_json = true

return department
