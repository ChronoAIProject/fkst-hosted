local core = require("core")
local saga = require("workflow.saga")

local spec = {
  consumes = { "devloop_substrate_ref_tick" },
  produces = { "github-proxy.github_pr_comment_request" },
  stall_window = "5m",
}

local function done(_event)
  return false
end

local function act(event)
  core.log_entry("substrate_ref_scan", event, "repo-management-plane", "tick")
  core.substrate_ref_scan()
end

return saga.department(spec, {
  done = done,
  act = act,
  name = "substrate_ref_scan",
})
