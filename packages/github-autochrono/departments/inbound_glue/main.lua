local core = require("core")

local M = {}

M.spec = {
  consumes = { "github-proxy.github_entity_changed" },
  produces = { "autochrono.issue" },
  stall_window = "30s",
}

function pipeline(event)
  local payload = event.payload or {}
  if payload.type ~= "issue" then
    return
  end

  raise("autochrono.issue", core.entity_to_issue(payload))
end

return M
