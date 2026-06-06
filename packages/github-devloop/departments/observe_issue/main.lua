local core = require("core")

local M = {}

M.spec = {
  consumes = { "github-proxy.github_entity_changed" },
  produces = {
    "consensus.proposal",
    "github-proxy.github_issue_label_request",
    "github-proxy.github_issue_comment_request",
  },
  fanout = { "github-proxy.github_entity_changed" },
  stall_window = "30s",
}

function pipeline(event)
  local issue = event.payload or {}
  if not core.is_supported_issue(issue) then
    core.log_entry("observe_issue", event, "unknown", issue.dedup_key)
    core.log_cas_decision("observe_issue", "unknown", { state = nil, version = nil }, "unmanaged", "thinking", "skip-foreign(proposal_id)", "unsupported event payload")
    return
  end

  local proposal_id = core.proposal_id(issue.repo, issue.number)
  core.log_entry("observe_issue", event, proposal_id, issue.dedup_key)
  local lock_key = core.observe_lock_key(issue.repo, issue.number)
  with_lock(lock_key, function()
    core.assert_trusted_bot_configured()

    local state_view = exec_sync({ cmd = core.gh_issue_view_state_cmd(issue.repo, issue.number), timeout = 30 })
    if state_view.exit_code ~= 0 then
      error("github-devloop: gh issue state view failed: " .. tostring(state_view.stderr))
    end

    local current = core.parse_issue_view_state(state_view.stdout)
    if current.state ~= "OPEN" then
      core.log_cas_decision("observe_issue", proposal_id, { state = nil, version = nil }, "unmanaged", "thinking", "skip-advanced-or-diverged", "issue is not open")
      return
    end
    if not core.is_opted_in(current.labels) then
      core.log_cas_decision("observe_issue", proposal_id, { state = nil, version = nil }, "unmanaged", "thinking", "skip-not-opted-in", "fkst-dev:enabled label is absent")
      return
    end
    core.log_forged_markers("observe_issue", proposal_id, current.comments)
    local state = core.current_state(current.comments, proposal_id)
    if state.state ~= nil then
      if state.state == "thinking" then
        core.log_cas_decision("observe_issue", proposal_id, state, "unmanaged", "thinking", "skip-idempotent(already at to_state)", "trusted thinking state marker is already visible")
      end
      if not core.state_label_hint_matches(current.labels, state.state) then
        local label_request = core.build_reconcile_state_label_request(issue.repo, issue.number, proposal_id, state.state, state.version, issue.source_ref)
        local add_labels, remove_labels = core.state_label_changes(state.state)
        core.log_apply("observe_issue", proposal_id, state.state, state.version, { add = add_labels, remove = remove_labels }, {
          "github-proxy.github_issue_label_request",
        })
        core.log_raise("observe_issue", proposal_id, "github-proxy.github_issue_label_request", label_request)
      end
      if state.state == "thinking" then
        return
      end
    end
    local transition = core.versioned_transition_status(state, { "unmanaged" }, "thinking", issue.dedup_key)
    if transition == "stale" then
      core.log_cas_decision("observe_issue", proposal_id, state, "unmanaged", "thinking", core.cas_outcome(state, transition, issue.dedup_key), "current marker is not an unmanaged start")
      return
    end
    if transition == "pending" then
      core.log_cas_decision("observe_issue", proposal_id, state, "unmanaged", "thinking", core.cas_outcome(state, transition, issue.dedup_key), "unmanaged state marker pending for observe")
      error("github-devloop: unmanaged state marker pending for observe; retrying")
    end
    core.log_cas_decision("observe_issue", proposal_id, state, "unmanaged", "thinking", core.cas_outcome(state, transition, issue.dedup_key), "starting consensus for opted-in issue")

    local view = exec_sync({ cmd = core.gh_issue_view_body_cmd(issue.repo, issue.number), timeout = 30 })
    if view.exit_code ~= 0 then
      error("github-devloop: gh issue view failed: " .. tostring(view.stderr))
    end

    local proposal = core.build_proposal(issue, core.parse_issue_view_body(view.stdout))
    if not core.validate_proposal(proposal) then
      log.warn("github-devloop dept=observe_issue proposal_id=" .. tostring(proposal_id) .. " tag=SKIP reason=cannot-build-valid-proposal")
      return
    end

    local comment_request = core.build_observe_comment_request(issue, proposal)
    local label_request = core.build_thinking_label_request(issue, proposal)
    local add_labels, remove_labels = core.state_label_changes("thinking")
    core.log_apply("observe_issue", proposal_id, "thinking", proposal.dedup_key, { add = add_labels, remove = remove_labels }, {
      "consensus.proposal",
      "github-proxy.github_issue_comment_request",
      "github-proxy.github_issue_label_request",
    })
    core.log_raise("observe_issue", proposal_id, "consensus.proposal", proposal)
    core.log_raise("observe_issue", proposal_id, "github-proxy.github_issue_comment_request", comment_request)
    core.log_raise("observe_issue", proposal_id, "github-proxy.github_issue_label_request", label_request)
  end)
end

return M
