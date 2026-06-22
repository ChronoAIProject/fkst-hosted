local core = require("core")
local saga = require("workflow.saga")

local spec = {
  consumes = { "devloop_ensure_repo_tick" },
  produces = {},
  ephemeral = { "devloop_ensure_repo_tick" },
  retry = false,
  stall_window = "2m",
}

local function ensure_repo_done(_event)
  return false
end

local function act_ensure_repo(event)
  core.log_entry("ensure_repo", event, "repo-management-plane", "tick")
  core.ensure_repo()
end

return saga.department(spec, {
  done = ensure_repo_done,
  act = act_ensure_repo,
  name = "ensure_repo",
})
