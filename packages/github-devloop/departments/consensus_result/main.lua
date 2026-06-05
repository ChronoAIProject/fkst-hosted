local core = require("core")

local M = {}

M.spec = {
  consumes = { "consensus.consensus_reached" },
  produces = { "github-proxy.github_issue_label_request", "github-proxy.github_issue_comment_request" },
  fanout = { "consensus.consensus_reached" },
  stall_window = "30s",
}

function pipeline(event)
  local reached = event.payload or {}
  if not core.is_supported_result(reached) then
    return
  end

  local repo, issue_number = core.parse_proposal_id(reached.proposal_id)
  if repo == nil then
    return
  end

  local lock_key = core.result_lock_key(reached.proposal_id)
  if lock_key == nil then
    return
  end

  with_lock(lock_key, function()
    local view = exec_sync({ cmd = core.gh_issue_view_result_cmd(repo, issue_number), timeout = 30 })
    if view.exit_code ~= 0 then
      error("github-devloop: gh issue result view failed: " .. tostring(view.stderr))
    end

    local current = core.parse_issue_view_result(view.stdout)
    local label_request = core.build_result_label_request(repo, issue_number, reached)
    local has_label = core.has_label(current.labels, label_request.add_labels[1])
    local has_stale_label = false
    for _, label in ipairs(label_request.remove_labels) do
      if core.has_label(current.labels, label) then
        has_stale_label = true
      end
    end
    local has_marker = core.has_result_marker(current.comments, reached.proposal_id, reached.decision, reached.dedup_key)

    -- Self-heal: re-raise the label request if the target is missing OR any stale state label
    -- is still present (the fkst-dev:<state> labels must stay mutually exclusive).
    if not has_label or has_stale_label then
      raise("github-proxy.github_issue_label_request", label_request)
    end
    if not has_marker then
      raise("github-proxy.github_issue_comment_request", core.build_result_comment_request(repo, issue_number, reached))
    end
  end)
end

return M
