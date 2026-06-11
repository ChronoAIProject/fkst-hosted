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
  if type(reached) == "table" and reached.schema == "consensus.consensus_reached.v1"
    and reached.decision == "reject" then
    core.log_entry("consensus_result", event, tostring(reached.proposal_id or "unknown"), reached.dedup_key)
    core.log_cas_decision("consensus_result", tostring(reached.proposal_id or "unknown"), { state = nil, version = nil }, "thinking", "ready", "skip-unsupported(decision)", "issue consensus does not support reject")
    return
  end
  if not core.is_supported_result(reached) then
    core.log_entry("consensus_result", event, "unknown", reached.dedup_key)
    core.log_cas_decision("consensus_result", "unknown", { state = nil, version = nil }, "thinking", "ready", "skip-foreign(proposal_id)", "unsupported event payload")
    return
  end

  core.log_entry("consensus_result", event, reached.proposal_id, reached.dedup_key)
  local repo, issue_number = core.parse_proposal_id(reached.proposal_id)
  if repo == nil then
    core.log_cas_decision("consensus_result", reached.proposal_id, { state = nil, version = nil }, "thinking", "ready", "skip-foreign(proposal_id)", "proposal_id is outside github-devloop")
    return
  end

  local lock_key = core.result_lock_key(reached.proposal_id)
  if lock_key == nil then
    core.log_cas_decision("consensus_result", reached.proposal_id, { state = nil, version = nil }, "thinking", "ready", "skip-foreign(proposal_id)", "no transition lock key")
    return
  end

  with_lock(lock_key, function()
    core.assert_trusted_bot_configured()

    local view = core.gh_exec({ cmd = core.gh_issue_view_result_cmd(repo, issue_number), timeout = 30 })
    if view.exit_code ~= 0 then
      error("github-devloop: gh issue result view failed: " .. tostring(view.stderr))
    end

    local current = core.parse_issue_view_result(view.stdout)
    local to_state = "ready"
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
    local gate = core.dependency_gate(repo, issue_number)
    local dependency_comment_request = nil
    local dependency_label_request = nil
    if not gate.ok then
      local version = tostring(reached.dedup_key)
      local marker = gate.kind == "cycle"
        and core.dependency_cycle_marker(reached.proposal_id, version)
        or core.dependency_wait_marker(reached.proposal_id, version, gate.unmet)
      dependency_comment_request = {
        schema = "github-proxy.v1",
        repo = repo,
        issue_number = issue_number,
        body = "github-devloop dependency hold: " .. tostring(gate.kind) .. "\n\nReason: " .. tostring(gate.reason) .. "\n\n" .. marker,
        dedup_key = core._dedup_key({ "dependency", "comment", tostring(reached.proposal_id), version, tostring(gate.kind) }),
        source_ref = core.normalize_source_ref(reached.source_ref),
      }
      dependency_label_request = core.build_label_request(
        repo,
        issue_number,
        { core._blocked_on_dependency_label },
        {},
        core._dedup_key({ "dependency", "label", "hold", tostring(reached.proposal_id), version, tostring(gate.kind) }),
        reached.source_ref
      )
    end
    table.insert(label_request.remove_labels, core._blocked_on_dependency_label)
    local add_labels, remove_labels = core.state_label_changes(to_state)
    local raised = {
      "github-proxy.github_issue_comment_request",
      "github-proxy.github_issue_label_request",
    }
    if gate.ok then
      table.insert(raised, "devloop_ready")
    else
      table.insert(raised, "github-proxy.github_issue_comment_request")
      table.insert(raised, "github-proxy.github_issue_label_request")
    end
    core.log_apply("consensus_result", reached.proposal_id, to_state, reached.dedup_key, { add = add_labels, remove = remove_labels }, raised)
    core.log_raise("consensus_result", reached.proposal_id, "github-proxy.github_issue_comment_request", comment_request)
    core.log_raise("consensus_result", reached.proposal_id, "github-proxy.github_issue_label_request", label_request)
    if not gate.ok then
      core.log_cas_decision("consensus_result", reached.proposal_id, state, "ready", "implementing", "hold-dependency", gate.reason)
      core.log_raise("consensus_result", reached.proposal_id, "github-proxy.github_issue_comment_request", dependency_comment_request)
      core.log_raise("consensus_result", reached.proposal_id, "github-proxy.github_issue_label_request", dependency_label_request)
      return
    end
    core.log_raise("consensus_result", reached.proposal_id, "devloop_ready", core.build_devloop_ready_payload(reached))
  end)
end

return M
