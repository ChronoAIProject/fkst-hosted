local core = require("core")

local M = {}

M.spec = {
  consumes = { "devloop_observe_tick" },
  produces = { "github-proxy.github_issue_create_request", "devloop_merge_queue_tick" },
  retry = false,
  stall_window = "2m",
}

function pipeline(event)
  core.log_entry("observability", event, "github-devloop/observability", "tick")
  core.observe_devloop_entities(event)
end

pipeline = core.wrap_pipeline_failure("observability", pipeline)

return M
