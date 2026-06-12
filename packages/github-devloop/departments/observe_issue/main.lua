local core = require("core")

local M = {}

M.spec = {
  consumes = { "github-proxy.github_entity_changed" },
  produces = {
    "consensus.proposal",
    "github-proxy.github_issue_label_request",
    "github-proxy.github_issue_comment_request",
    "github-proxy.github_pr_comment_request",
    "devloop_ready",
    "devloop_reviewing",
    "devloop_fixing",
    "devloop_decompose",
    "devloop_merge_ready",
    "devloop_reconcile",
    "devloop_review_reconcile",
    "devloop_timeout_reconcile",
  },
  fanout = { "github-proxy.github_entity_changed" },
  stall_window = "30s",
}

local observe_replay_states = {
  thinking = true,
  ready = true,
  ["pr-open"] = true,
  fixing = true,
  ["review-meta"] = true,
  blocked = true,
  ["impl-failed"] = true,
}

local function replay_or_timeout(issue, proposal_id, current, link, snapshot, state, event_ts, issue_state)
  local row = core.restart_transition_row(state.state)
  local facts = {
    proposal_id = proposal_id,
    current = current,
    link = link,
    snapshot = snapshot,
    event_ts = event_ts,
  }
  if observe_replay_states[state.state]
    and core.replay_from_table("observe_issue", issue, state, row, facts) then
    return true
  end
  if observe_replay_states[state.state] then
    return false
  end
  if issue_state == nil
    or issue_state.state ~= state.state
    or tostring(issue_state.version or "") ~= tostring(state.version or "") then
    return false
  end
  return core.maybe_timeout_redrive_from_table("observe_issue", issue, state, row, facts)
end

local function maybe_apply_issue_rereview_command(issue, proposal_id, current, state, event_ts)
  local command = core.operator_command_fact(current.comments, "rereview")
  if command == nil then
    return false
  end
  if core.has_operator_command_response(current.comments, command) then
    core.log_cas_decision("observe_issue", proposal_id, state, "stalled-thinking", "thinking", "skip-idempotent(command-response-visible)", "operator command response marker is already visible")
    return false
  end
  if state.state ~= "thinking" or not core.has_thinking_converge_replay(current, proposal_id, state, issue.source_ref) then
    core.log_cas_decision("observe_issue", proposal_id, state, "thinking-converge", "thinking", "refused(invalid-state)", "operator rereview requires thinking converge")
    local refusal = core.build_operator_issue_command_refusal_request(
      issue.repo,
      issue.number,
      command,
      "rereview requires thinking converge state",
      issue.source_ref
    )
    core.log_raise("observe_issue", proposal_id, "github-proxy.github_issue_comment_request", refusal)
    return true
  end

  local proposal = core.build_thinking_replay_proposal(issue, proposal_id, state, current, event_ts)
  if proposal == nil then
    core.log_cas_decision("observe_issue", proposal_id, state, "stalled-thinking", "thinking", "refused(cannot-rebuild-proposal)", "operator rereview could not rebuild thinking proposal")
    local refusal = core.build_operator_issue_command_refusal_request(
      issue.repo,
      issue.number,
      command,
      "rereview could not rebuild the current thinking proposal",
      issue.source_ref
    )
    core.log_raise("observe_issue", proposal_id, "github-proxy.github_issue_comment_request", refusal)
    return true
  end

  local comment_request = core.build_operator_issue_rereview_comment_request(
    issue.repo,
    issue.number,
    command,
    proposal,
    issue.source_ref
  )
  core.log_cas_decision("observe_issue", proposal_id, state, "stalled-thinking", "thinking", "applied(operator-rereview)", "trusted operator command requested issue rereview")
  core.log_apply("observe_issue", proposal_id, "thinking", proposal.dedup_key, { add = {}, remove = {} }, {
    "github-proxy.github_issue_comment_request",
    "consensus.proposal",
  })
  core.log_raise("observe_issue", proposal_id, "github-proxy.github_issue_comment_request", comment_request)
  core.log_raise("observe_issue", proposal_id, "consensus.proposal", proposal)
  return true
end

local function raise_stale_dependency_label_clear(issue, proposal_id, state, labels)
  if state.state == "ready" or not core.has_label(labels, core._blocked_on_dependency_label) then
    return false
  end
  core.log_apply("observe_issue", proposal_id, state.state, state.version, { add = {}, remove = { core._blocked_on_dependency_label } }, {
    "github-proxy.github_issue_label_request",
  })
  core.log_raise("observe_issue", proposal_id, "github-proxy.github_issue_label_request", core.build_label_request(
    issue.repo,
    issue.number,
    {},
    { core._blocked_on_dependency_label },
    core._dedup_key({ "dependency", "label", "clear", tostring(proposal_id), tostring(state.version or "unversioned") }),
    issue.source_ref
  ))
  return true
end

local function maybe_apply_issue_reready_command(issue, proposal_id, current, state)
  local command = core.operator_command_fact(current.comments, "reready")
  if command == nil then
    return false
  end
  if core.has_operator_command_response(current.comments, command) then
    core.log_cas_decision("observe_issue", proposal_id, state, "ready", "ready", "skip-idempotent(command-response-visible)", "operator command response marker is already visible")
    return false
  end
  if state.state ~= "ready" then
    core.log_cas_decision("observe_issue", proposal_id, state, "ready", "ready", "refused(invalid-state)", "operator reready requires ready state")
    local refusal = core.build_operator_issue_command_refusal_request(
      issue.repo,
      issue.number,
      command,
      "reready requires ready state",
      issue.source_ref
    )
    core.log_raise("observe_issue", proposal_id, "github-proxy.github_issue_comment_request", refusal)
    return true
  end
  core.replay_from_table("observe_issue", issue, state, core.restart_transition_row("ready"), {
    proposal_id = proposal_id,
    current = current,
    command = command,
  })
  return true
end

local function has_unmet_blocker(gate, blocker_number)
  if type(gate) ~= "table" or type(gate.unmet) ~= "table" then
    return false
  end
  for _, number in ipairs(gate.unmet) do
    if tonumber(number) == tonumber(blocker_number) then
      return true
    end
  end
  return false
end

local function maybe_apply_issue_dependency_waiver_command(issue, proposal_id, current, state)
  local command = core.operator_command_fact(current.comments, "dependency-waiver")
  if command == nil then
    return false
  end
  if core.has_operator_command_response(current.comments, command) then
    core.log_cas_decision("observe_issue", proposal_id, state, "ready", "ready", "skip-idempotent(command-response-visible)", "operator command response marker is already visible")
    return false
  end
  if state.state ~= "ready" then
    core.log_cas_decision("observe_issue", proposal_id, state, "ready", "ready", "refused(invalid-state)", "operator dependency waiver requires ready state")
    local refusal = core.build_operator_issue_command_refusal_request(
      issue.repo,
      issue.number,
      command,
      "dependency-waiver requires ready state",
      issue.source_ref
    )
    core.log_raise("observe_issue", proposal_id, "github-proxy.github_issue_comment_request", refusal)
    return true
  end

  local blocker_number = command.blocker_number
  local gate = core.dependency_gate(issue.repo, issue.number, {
    proposal_id = proposal_id,
    version = state.version,
    comments = current.comments,
  })
  if gate.kind ~= "waiting"
    or gate.reason ~= "dependency-waiver-required"
    or not has_unmet_blocker(gate, blocker_number) then
    core.log_cas_decision("observe_issue", proposal_id, state, "ready", "ready", "refused(invalid-dependency-waiver)", "operator dependency waiver requires a matching completed blocker without merged marker")
    local refusal = core.build_operator_issue_command_refusal_request(
      issue.repo,
      issue.number,
      command,
      "dependency-waiver requires a matching completed blocker without merged marker",
      issue.source_ref
    )
    core.log_raise("observe_issue", proposal_id, "github-proxy.github_issue_comment_request", refusal)
    return true
  end

  local comment_request = core.build_operator_issue_dependency_waiver_comment_request(
    issue.repo,
    issue.number,
    command,
    proposal_id,
    state.version,
    blocker_number,
    issue.source_ref
  )
  local payload = core.build_devloop_ready_payload({
    proposal_id = proposal_id,
    dedup_key = state.version,
    source_ref = issue.source_ref,
  })
  core.log_cas_decision("observe_issue", proposal_id, state, "ready", "ready", "applied(operator-dependency-waiver)", "trusted operator command created dependency waiver")
  core.log_apply("observe_issue", proposal_id, nil, nil, { add = {}, remove = {} }, {
    "github-proxy.github_issue_comment_request",
    "devloop_ready",
  })
  core.log_raise("observe_issue", proposal_id, "github-proxy.github_issue_comment_request", comment_request)
  core.log_raise("observe_issue", proposal_id, "devloop_ready", payload)
  return true
end

local function maybe_apply_issue_reimplement_command(issue, proposal_id, current, state)
  local command = core.operator_command_fact(current.comments, "reimplement")
  if command == nil then
    return false
  end
  if core.has_operator_command_response(current.comments, command) then
    core.log_cas_decision("observe_issue", proposal_id, state, "impl-failed", "implementing", "skip-idempotent(command-response-visible)", "operator command response marker is already visible")
    return false
  end
  if state.state ~= "impl-failed" then
    core.log_cas_decision("observe_issue", proposal_id, state, "impl-failed", "implementing", "refused(invalid-state)", "operator reimplement requires impl-failed state")
    local refusal = core.build_operator_issue_command_refusal_request(
      issue.repo,
      issue.number,
      command,
      "reimplement requires impl-failed state",
      issue.source_ref
    )
    core.log_raise("observe_issue", proposal_id, "github-proxy.github_issue_comment_request", refusal)
    return true
  end

  local attempt = 1
  local failure = core.impl_failure_fact(current.comments, proposal_id, state.version)
  if failure ~= nil then
    attempt = tonumber(failure.attempt or 1) + 1
  end
  local payload = core.build_devloop_ready_payload({
    proposal_id = proposal_id,
    dedup_key = state.version,
    source_ref = issue.source_ref,
    impl_retry_attempt = attempt,
  })
  local comment_request = core.build_operator_issue_reimplement_comment_request(
    issue.repo,
    issue.number,
    command,
    attempt,
    issue.source_ref
  )
  core.log_cas_decision("observe_issue", proposal_id, state, "impl-failed", "implementing", "applied(operator-reimplement)", "trusted operator command requested implementation retry")
  core.log_apply("observe_issue", proposal_id, nil, nil, { add = {}, remove = {} }, {
    "github-proxy.github_issue_comment_request",
    "devloop_ready",
  })
  core.log_raise("observe_issue", proposal_id, "github-proxy.github_issue_comment_request", comment_request)
  core.log_raise("observe_issue", proposal_id, "devloop_ready", payload)
  return true
end

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

    local state_view = core.fetch_issue_view_state(issue.repo, issue.number, issue.updated_at)
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
    local link = core.pr_link_fact(current.comments, proposal_id)
    local snapshot = core.linked_entity_snapshot(issue.repo, proposal_id, current.comments)
    snapshot.fresh = true
    local state = snapshot.state
    local issue_state = core.current_state(current.comments, proposal_id)
    if state.state ~= nil then
      if maybe_apply_issue_rereview_command(issue, proposal_id, current, state, event.ts) then
        return
      end
      if maybe_apply_issue_reready_command(issue, proposal_id, current, state) then
        return
      end
      if maybe_apply_issue_dependency_waiver_command(issue, proposal_id, current, state) then
        return
      end
      if maybe_apply_issue_reimplement_command(issue, proposal_id, current, state) then
        return
      end
      if not core.state_label_hint_matches(current.labels, state.state) then
        local label_request = core.build_reconcile_state_label_request(issue.repo, issue.number, proposal_id, state.state, state.version, issue.source_ref)
        local add_labels, remove_labels = core.state_label_changes(state.state)
        core.log_apply("observe_issue", proposal_id, state.state, state.version, { add = add_labels, remove = remove_labels }, {
          "github-proxy.github_issue_label_request",
        })
        core.log_raise("observe_issue", proposal_id, "github-proxy.github_issue_label_request", label_request)
      end
      raise_stale_dependency_label_clear(issue, proposal_id, state, current.labels)
      if replay_or_timeout(issue, proposal_id, current, link, snapshot, state, event.ts, issue_state) then
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

    issue.content_fetch = core.context_fetch_ref_from_bundle({
      dept = "observe_issue",
      repo = issue.repo,
      issue_number = issue.number,
      proposal_id = proposal_id,
      version = issue.dedup_key,
      tick = event.ts,
    })
    local proposal = core.build_board_proposal(issue, event.ts)
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
