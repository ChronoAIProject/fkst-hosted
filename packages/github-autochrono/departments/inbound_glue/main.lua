local core = require("core")
local saga = require("workflow.saga")

local spec = {
  consumes = { "github-proxy.github_entity_changed" },
  produces = { "autochrono.issue" },
  fanout = { "github-proxy.github_entity_changed" },
  stall_window = "30s",
}

local function is_issue_entity(event)
  local payload = event.payload or {}
  return payload.type == "issue"
end

local function glue_done(_event)
  return false
end

local function act_glue(event)
  local payload = event.payload or {}
  raise("autochrono.issue", core.entity_to_issue(payload))
end

return saga.department(spec, {
  accept = is_issue_entity,
  done = glue_done,
  act = act_glue,
  name = "inbound_glue",
})
