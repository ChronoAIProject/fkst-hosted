local core = require("core")

local M = {}

M.spec = {
  consumes = { "consensus.consensus_unresolved" },
  produces = {
    "consensus.proposal",
    "github-proxy.github_issue_label_request",
    "github-proxy.github_issue_comment_request",
  },
  fanout = { "consensus.consensus_unresolved" },
  stall_window = "30s",
}

function pipeline(event)
  local unresolved = event.payload or {}
  if not core.is_supported_unresolved(unresolved) then
    return
  end

  local repo, issue_number = core.parse_proposal_id(unresolved.proposal_id)
  if repo == nil then
    return
  end

  local lock_key = core.loop_lock_key(unresolved.proposal_id)
  if lock_key == nil then
    return
  end

  with_lock(lock_key, function()
    local view = exec_sync({ cmd = core.gh_issue_view_loop_cmd(repo, issue_number), timeout = 30 })
    if view.exit_code ~= 0 then
      error("github-devloop: gh issue loop view failed: " .. tostring(view.stderr))
    end

    local current = core.parse_issue_view_loop(view.stdout)
    -- A decision terminal (ready/blocked) means consensus_result already decided: this unresolved is
    -- stale, skip. (A lingering thinking label next to a decision label is consensus_result's to heal.)
    if core.has_decision_terminal_label(current.labels) then
      return
    end
    -- The state label may not have landed yet (observe's thinking label request is async). Do NOT
    -- ack-drop a legitimate unresolved as if the issue were unmanaged: only proceed once thinking or
    -- stuck is visible, otherwise error so reliable delivery retries until the label appears.
    if not core.has_thinking_label(current.labels) and not core.has_stuck_label(current.labels) then
      error("github-devloop: issue state label not yet visible for unresolved; retrying")
    end

    local budget = core.loop_budget()
    if core.has_loop_marker_dedup(current.comments, unresolved.proposal_id, unresolved.dedup_key) then
      return
    end

    local event_n = core.parse_loop_round_from_dedup(unresolved.dedup_key)
    local marker_n = core.loop_count_from_github_markers(current.comments, unresolved.proposal_id)
    local current_n = math.max(event_n, marker_n)
    if current_n >= budget then
      if not core.has_stuck_marker(current.comments, unresolved.proposal_id, budget, unresolved.dedup_key) then
        raise("github-proxy.github_issue_comment_request", core.build_stuck_comment_request(repo, issue_number, unresolved, budget))
      end
      if not core.has_stuck_label(current.labels) or core.has_thinking_label(current.labels) then
        raise("github-proxy.github_issue_label_request", core.build_stuck_label_request(repo, issue_number, unresolved, budget))
      end
      return
    end

    local next_n = current_n + 1
    if core.has_loop_marker_round(current.comments, unresolved.proposal_id, next_n) then
      return
    end

    if next_n >= budget then
      raise("github-proxy.github_issue_comment_request", core.build_stuck_comment_request(repo, issue_number, unresolved, next_n))
      raise("github-proxy.github_issue_label_request", core.build_stuck_label_request(repo, issue_number, unresolved, next_n))
      return
    end

    local proposal = core.build_loop_proposal(repo, issue_number, current, unresolved.source_ref, next_n)
    if not core.validate_proposal(proposal) then
      log.warn("github-devloop: cannot build a valid loop proposal; skipping")
      return
    end

    raise("consensus.proposal", proposal)
    raise("github-proxy.github_issue_comment_request", core.build_loop_comment_request(repo, issue_number, unresolved, next_n))
  end)
end

return M
