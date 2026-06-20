local core, saga = require("core"), require("std.saga")

local spec = {
  consumes = { "devloop_pr_terminal" },
  produces = {
    "github-proxy.github_pr_comment_request",
    "github-proxy.github_issue_comment_request",
    "github-proxy.github_issue_label_request",
  },
  stall_window = "30s",
  retry = { max_attempts = 12, base = "5s", cap = "30s" },
}

local function done(_event)
  return false
end

local function act(event)
  local payload = event.payload or {}
  core.log_entry("on_pr_terminal", event, payload.proposal_id, payload.dedup_key)
  if not core.event_queue_matches(event, "devloop_pr_terminal") then
    error("github-devloop: error_class=on_pr_terminal_unsupported_queue")
  end
  if not core.is_supported_pr_terminal(payload) then
    error("github-devloop: error_class=on_pr_terminal_unsupported_payload")
  end
  core.assert_trusted_bot_configured()
  core.on_pr_terminal(payload)
end

return saga.department(spec, {
  done = done,
  act = act,
  wrap = core.wrap_pipeline_failure,
  name = "on_pr_terminal",
})
