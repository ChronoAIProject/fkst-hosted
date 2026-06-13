local core = require("core")

local M = {}

M.spec = {
  consumes = { "devloop_substrate_ref_tick" },
  produces = {},
  ephemeral = { "devloop_substrate_ref_tick" },
  retry = false,
  stall_window = "5m",
}

function pipeline(event)
  core.log_entry("substrate_ref_scan", event, "repo-management-plane", "tick")
  core.substrate_ref_scan()
end

return M
