local core = require("core")
local saga = require("std.saga")

local spec = {
  consumes = { "devloop_observe_tick" },
  produces = { "github-proxy.github_issue_create_request", "devloop_merge_queue_tick" },
  graph_json = true,
  retry = false,
  stall_window = "2m",
}

local function observability_done(_event)
  return false
end

local function act_observability(event)
  core.log_entry("observability", event, "github-devloop/observability", "tick")
  core.observe_devloop_entities(event)
end

return saga.department(spec, {
  done = observability_done,
  act = act_observability,
  wrap = core.wrap_pipeline_failure,
  name = "observability",
})
