local core = require("core")

local M = {}

M.spec = {
  consumes = { "consensus.consensus_converge" },
  produces = {
    "devloop_stuck",
    "consensus.proposal",
    "github-proxy.github_issue_label_request",
    "github-proxy.github_issue_comment_request",
  },
  fanout = { "consensus.consensus_converge" },
  stall_window = "30s",
}

function pipeline(event)
  local unresolved = event.payload or {}
  if not core.is_supported_unresolved(unresolved) then
    core.log_entry("loop", event, "unknown", unresolved.dedup_key)
    core.log_cas_decision("loop", "unknown", { state = nil, version = nil }, "thinking", "stuck", "skip-foreign(proposal_id)", "unsupported event payload")
    return
  end

  core.log_entry("loop", event, unresolved.proposal_id, unresolved.dedup_key)
  local repo, issue_number = core.parse_proposal_id(unresolved.proposal_id)
  if repo == nil then
    core.log_cas_decision("loop", unresolved.proposal_id, { state = nil, version = nil }, "thinking", "stuck", "skip-foreign(proposal_id)", "proposal_id is outside github-devloop")
    return
  end

  local lock_key = core.loop_lock_key(unresolved.proposal_id)
  if lock_key == nil then
    core.log_cas_decision("loop", unresolved.proposal_id, { state = nil, version = nil }, "thinking", "stuck", "skip-foreign(proposal_id)", "no transition lock key")
    return
  end

  with_lock(lock_key, function()
    core.assert_trusted_bot_configured()

    local view = exec_sync({ cmd = core.gh_issue_view_loop_cmd(repo, issue_number), timeout = 30 })
    if view.exit_code ~= 0 then
      error("github-devloop: gh issue loop view failed: " .. tostring(view.stderr))
    end

    local current = core.parse_issue_view_loop(view.stdout)
    core.log_forged_markers("loop", unresolved.proposal_id, current.comments)
    local state = core.current_state(current.comments, unresolved.proposal_id)
    local transition = core.versioned_transition_status(state, { "thinking" }, "stuck", unresolved.dedup_key)
    if transition == "idempotent" or transition == "stale" then
      core.log_cas_decision("loop", unresolved.proposal_id, state, "thinking", "stuck", core.cas_outcome(state, transition, unresolved.dedup_key), "unresolved event cannot advance current marker")
      return
    end
    if transition == "pending" then
      core.log_cas_decision("loop", unresolved.proposal_id, state, "thinking", "stuck", core.cas_outcome(state, transition, unresolved.dedup_key), "thinking state marker not yet visible")
      error("github-devloop: thinking state marker not yet visible for unresolved; retrying")
    end

    local budget = core.loop_budget()
    if core.has_loop_marker_dedup(current.comments, unresolved.proposal_id, unresolved.dedup_key) then
      core.log_cas_decision("loop", unresolved.proposal_id, state, "thinking", "stuck", "skip-idempotent(already at to_state)", "loop marker for incoming dedup is already visible")
      return
    end

    local event_n = core.parse_loop_round_from_dedup(unresolved.dedup_key)
    local marker_n = core.loop_count_from_github_markers(current.comments, unresolved.proposal_id)
    local current_n = math.max(event_n, marker_n)
    if current_n >= budget then
      core.log_cas_decision("loop", unresolved.proposal_id, state, "thinking", "stuck", core.cas_outcome(state, transition, unresolved.dedup_key), "loop budget exhausted at round " .. tostring(current_n))
      local comment_request = core.build_stuck_comment_request(repo, issue_number, unresolved, budget)
      local label_request = core.build_stuck_label_request(repo, issue_number, unresolved, budget)
      local add_labels, remove_labels = core.state_label_changes("stuck")
      core.log_apply("loop", unresolved.proposal_id, "stuck", unresolved.dedup_key, { add = add_labels, remove = remove_labels }, {
        "github-proxy.github_issue_comment_request",
        "github-proxy.github_issue_label_request",
        "devloop_stuck",
      })
      core.log_raise("loop", unresolved.proposal_id, "github-proxy.github_issue_comment_request", comment_request)
      core.log_raise("loop", unresolved.proposal_id, "github-proxy.github_issue_label_request", label_request)
      core.log_raise("loop", unresolved.proposal_id, "devloop_stuck", core.build_devloop_stuck_payload(unresolved, current_n))
      return
    end

    local next_n = current_n + 1
    if core.has_loop_marker_round(current.comments, unresolved.proposal_id, next_n) then
      core.log_cas_decision("loop", unresolved.proposal_id, state, "thinking", "stuck", "skip-idempotent(already at to_state)", "loop marker for next round is already visible")
      return
    end

    if next_n >= budget then
      core.log_cas_decision("loop", unresolved.proposal_id, state, "thinking", "stuck", core.cas_outcome(state, transition, unresolved.dedup_key), "next loop round reaches budget " .. tostring(next_n))
      local comment_request = core.build_stuck_comment_request(repo, issue_number, unresolved, next_n)
      local label_request = core.build_stuck_label_request(repo, issue_number, unresolved, next_n)
      local add_labels, remove_labels = core.state_label_changes("stuck")
      core.log_apply("loop", unresolved.proposal_id, "stuck", unresolved.dedup_key, { add = add_labels, remove = remove_labels }, {
        "github-proxy.github_issue_comment_request",
        "github-proxy.github_issue_label_request",
        "devloop_stuck",
      })
      core.log_raise("loop", unresolved.proposal_id, "github-proxy.github_issue_comment_request", comment_request)
      core.log_raise("loop", unresolved.proposal_id, "github-proxy.github_issue_label_request", label_request)
      core.log_raise("loop", unresolved.proposal_id, "devloop_stuck", core.build_devloop_stuck_payload(unresolved, next_n))
      return
    end

    local proposal = core.build_loop_proposal(repo, issue_number, current, unresolved.source_ref, next_n)
    if not core.validate_proposal(proposal) then
      log.warn("github-devloop dept=loop proposal_id=" .. tostring(unresolved.proposal_id) .. " tag=SKIP reason=cannot-build-valid-loop-proposal")
      return
    end

    core.log_cas_decision("loop", unresolved.proposal_id, state, "thinking", "stuck", core.cas_outcome(state, transition, unresolved.dedup_key), "raising loop proposal round " .. tostring(next_n))
    local comment_request = core.build_loop_comment_request(repo, issue_number, unresolved, next_n)
    core.log_apply("loop", unresolved.proposal_id, nil, nil, { add = {}, remove = {} }, {
      "consensus.proposal",
      "github-proxy.github_issue_comment_request",
    })
    core.log_raise("loop", unresolved.proposal_id, "consensus.proposal", proposal)
    core.log_raise("loop", unresolved.proposal_id, "github-proxy.github_issue_comment_request", comment_request)
  end)
end

return M
