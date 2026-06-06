local core = require("core")

local M = {}

M.spec = {
  consumes = { "consensus.consensus_reached" },
  produces = {
    "github-proxy.github_issue_label_request",
    "github-proxy.github_issue_comment_request",
    "devloop_ready",
  },
  fanout = { "consensus.consensus_reached" },
  stall_window = "30s",
}

function pipeline(event)
  local reached = event.payload or {}
  if not core.is_supported_result(reached) then
    core.log_entry("consensus_result", event, "unknown", reached.dedup_key)
    core.log_cas_decision("consensus_result", "unknown", { state = nil, version = nil }, "thinking", "ready|blocked", "skip-foreign(proposal_id)", "unsupported event payload")
    return
  end

  core.log_entry("consensus_result", event, reached.proposal_id, reached.dedup_key)
  local repo, issue_number = core.parse_proposal_id(reached.proposal_id)
  if repo == nil then
    core.log_cas_decision("consensus_result", reached.proposal_id, { state = nil, version = nil }, "thinking", "ready|blocked", "skip-foreign(proposal_id)", "proposal_id is outside github-devloop")
    return
  end

  local lock_key = core.result_lock_key(reached.proposal_id)
  if lock_key == nil then
    core.log_cas_decision("consensus_result", reached.proposal_id, { state = nil, version = nil }, "thinking", "ready|blocked", "skip-foreign(proposal_id)", "no transition lock key")
    return
  end

  with_lock(lock_key, function()
    core.assert_trusted_bot_configured()

    local view = exec_sync({ cmd = core.gh_issue_view_result_cmd(repo, issue_number), timeout = 30 })
    if view.exit_code ~= 0 then
      error("github-devloop: gh issue result view failed: " .. tostring(view.stderr))
    end

    local current = core.parse_issue_view_result(view.stdout)
    local to_state = reached.decision == "approve" and "ready" or "blocked"
    core.log_forged_markers("consensus_result", reached.proposal_id, current.comments)
    local state = core.current_state(current.comments, reached.proposal_id)
    local transition = core.versioned_transition_status(state, { "thinking" }, to_state, reached.dedup_key)
    if transition == "idempotent" or transition == "stale" then
      core.log_cas_decision("consensus_result", reached.proposal_id, state, "thinking", to_state, core.cas_outcome(state, transition, reached.dedup_key), "consensus result cannot advance current marker")
      return
    end
    if transition == "pending" then
      core.log_cas_decision("consensus_result", reached.proposal_id, state, "thinking", to_state, core.cas_outcome(state, transition, reached.dedup_key), "thinking state marker not yet visible")
      error("github-devloop: thinking state marker not yet visible for consensus result; retrying")
    end
    core.log_cas_decision("consensus_result", reached.proposal_id, state, "thinking", to_state, core.cas_outcome(state, transition, reached.dedup_key), "consensus decision=" .. tostring(reached.decision))

    local comment_request = core.build_result_comment_request(repo, issue_number, reached)
    local label_request = core.build_result_label_request(repo, issue_number, reached)
    local add_labels, remove_labels = core.state_label_changes(to_state)
    local raised = {
      "github-proxy.github_issue_comment_request",
      "github-proxy.github_issue_label_request",
    }
    if reached.decision == "approve" then
      table.insert(raised, "devloop_ready")
    end
    core.log_apply("consensus_result", reached.proposal_id, to_state, reached.dedup_key, { add = add_labels, remove = remove_labels }, raised)
    core.log_raise("consensus_result", reached.proposal_id, "github-proxy.github_issue_comment_request", comment_request)
    core.log_raise("consensus_result", reached.proposal_id, "github-proxy.github_issue_label_request", label_request)
    if reached.decision == "approve" then
      core.log_raise("consensus_result", reached.proposal_id, "devloop_ready", core.build_devloop_ready_payload(reached))
    end
  end)
end

return M
