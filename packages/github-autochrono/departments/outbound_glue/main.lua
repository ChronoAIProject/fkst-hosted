local core = require("core")
local saga = require("workflow.saga")

local spec = {
  consumes = { "autochrono.reply" },
  produces = { "github-proxy.github_issue_comment_request" },
  stall_window = "30s",
}

local function glue_done(_event)
  return false
end

local function act_glue(event)
  raise("github-proxy.github_issue_comment_request", core.reply_to_comment_request(event.payload or {}))
end

return saga.department(spec, {
  done = glue_done,
  act = act_glue,
  name = "outbound_glue",
})
