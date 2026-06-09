local core = require("core")

local M = {}

M.spec = {
  consumes = { "devloop_observe_tick" },
  produces = {},
  ephemeral = { "devloop_observe_tick" },
  retry = false,
  stall_window = "2m",
}

function pipeline(event)
  core.log_entry("observability", event, "github-devloop/observability", "tick")
  core.observe_devloop_entities()
end

return M
