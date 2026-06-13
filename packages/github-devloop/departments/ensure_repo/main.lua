local core = require("core")

local M = {}

M.spec = {
  consumes = { "devloop_ensure_repo_tick" },
  produces = {},
  ephemeral = { "devloop_ensure_repo_tick" },
  retry = false,
  stall_window = "2m",
}

function pipeline(event)
  core.log_entry("ensure_repo", event, "repo-management-plane", "tick")
  core.ensure_repo()
end

return M
