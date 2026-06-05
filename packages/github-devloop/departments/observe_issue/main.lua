local core = require("core")

local M = {}

M.spec = {
  consumes = { "github-proxy.github_entity_changed" },
  produces = { "consensus.proposal", "github-proxy.github_issue_label_request" },
  fanout = { "github-proxy.github_entity_changed" },
  stall_window = "30s",
}

function pipeline(event)
  local issue = event.payload or {}
  if not core.is_supported_issue(issue) then
    return
  end

  local lock_key = core.observe_lock_key(issue.repo, issue.number)
  with_lock(lock_key, function()
    local state_view = exec_sync({ cmd = core.gh_issue_view_state_cmd(issue.repo, issue.number), timeout = 30 })
    if state_view.exit_code ~= 0 then
      error("github-devloop: gh issue state view failed: " .. tostring(state_view.stderr))
    end

    local current = core.parse_issue_view_state(state_view.stdout)
    if current.state ~= "OPEN" then
      return
    end
    if not core.is_opted_in(current.labels) then
      return
    end

    local view = exec_sync({ cmd = core.gh_issue_view_body_cmd(issue.repo, issue.number), timeout = 30 })
    if view.exit_code ~= 0 then
      error("github-devloop: gh issue view failed: " .. tostring(view.stderr))
    end

    local proposal = core.build_proposal(issue, core.parse_issue_view_body(view.stdout))
    if not core.validate_proposal(proposal) then
      log.warn("github-devloop: cannot build a valid proposal; skipping")
      return
    end

    raise("consensus.proposal", proposal)
    raise("github-proxy.github_issue_label_request", core.build_thinking_label_request(issue, proposal))
  end)
end

return M
