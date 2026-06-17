local S = {}

function S.install(M)

local dependency_gate_rederive = true

function M.build_ready_split_canonicalized_comment_request(repo, issue_number, proposal_id, from_version, to_state, to_version, gate, source_ref)
  local markers = M.ready_split_canonicalized_marker(proposal_id, from_version, to_version, to_state, gate and gate.reason or "ready_split_rederive")
    .. "\n" .. M.state_marker(proposal_id, to_state, to_version, "ready-split-canonicalized")
  if to_state == "dependency_wait" then
    markers = markers .. "\n" .. M.dependency_wait_marker(proposal_id, to_version, gate and gate.unmet or {}, gate and gate.kind or "waiting", gate and gate.reason or "waiting-on-dependency")
  end
  return M.attach_issue_claim({
    schema = "github-proxy.v1",
    repo = repo,
    issue_number = issue_number,
    body = "github-devloop ready split canonicalized"
      .. "\n\n" .. M.comment_string("reason_inline_label") .. tostring(gate and gate.reason or "ready_split_rederive")
      .. "\n\n" .. markers,
    dedup_key = M._dedup_key({ "ready-split", "canonicalized", tostring(proposal_id), tostring(from_version), tostring(to_version) }),
    source_ref = M.normalize_source_ref(source_ref),
  }, source_ref)
end

function M.canonicalize_legacy_ready_dependency_wait(dept, issue, state, facts)
  if type(state) ~= "table" or state.state ~= "ready" then
    return false
  end
  local proposal_id = facts and facts.proposal_id or state.proposal_id
  local current = facts and facts.current
  local comments = current and current.comments
  if proposal_id == nil or type(comments) ~= "table" then
    return false
  end
  if M.ready_split_canonicalized_fact(comments, proposal_id, state.version) ~= nil then
    return false
  end
  if M.dependency_hold_fact(comments, proposal_id) == nil then
    return false
  end
  local gate = facts.dependency_gate or M.dependency_gate(issue.repo, issue.number, {
    proposal_id = proposal_id,
    version = state.version,
    comments = comments,
  })
  local to_state = gate.ok and "ready" or "dependency_wait"
  local to_version = M.ready_split_version(state.version)
  local raised = { "github-proxy.github_issue_comment_request" }
  local add_labels = {}
  local remove_labels = {}
  if to_state == "dependency_wait" then
    add_labels = { M._blocked_on_dependency_label }
    table.insert(raised, "github-proxy.github_issue_label_request")
  else
    remove_labels = { M._blocked_on_dependency_label }
    table.insert(raised, "devloop_ready")
    if M.has_label(current.labels, M._blocked_on_dependency_label) then
      table.insert(raised, "github-proxy.github_issue_label_request")
    end
  end
  M.log_cas_decision(dept, proposal_id, state, "ready", to_state, "applied(ready-split-canonicalized)", gate.reason or "ready_split_rederive")
  M.log_apply(dept, proposal_id, to_state, to_version, { add = add_labels, remove = remove_labels }, raised)
  M.log_raise(dept, proposal_id, "github-proxy.github_issue_comment_request", M.build_ready_split_canonicalized_comment_request(
    issue.repo,
    issue.number,
    proposal_id,
    state.version,
    to_state,
    to_version,
    gate,
    issue.source_ref
  ))
  if to_state == "dependency_wait" then
    M.log_raise(dept, proposal_id, "github-proxy.github_issue_label_request", M.build_label_request(
      issue.repo,
      issue.number,
      { M._blocked_on_dependency_label },
      {},
      M._dedup_key({ "dependency", "label", "hold", tostring(proposal_id), tostring(to_version), tostring(gate.kind) }),
      issue.source_ref
    ))
    return true
  end
  if M.has_label(current.labels, M._blocked_on_dependency_label) then
    M.log_raise(dept, proposal_id, "github-proxy.github_issue_label_request", M.build_label_request(
      issue.repo,
      issue.number,
      {},
      { M._blocked_on_dependency_label },
      M._dedup_key({ "dependency", "label", "clear", tostring(proposal_id), tostring(to_version) }),
      issue.source_ref
    ))
  end
  M.log_raise(dept, proposal_id, "devloop_ready", M.build_devloop_ready_payload({
    proposal_id = proposal_id,
    dedup_key = to_version,
    source_ref = issue.source_ref,
  }))
  return true
end

local function replay_fields(M, row, state, issue, proposal_id)
  return M.resolve_replay_payload_fields(row, state, {
    issue = issue,
    state = state,
    proposal_id = proposal_id,
  })
end

local function raise_dependency_release(M, dept, issue, proposal_id, state, current, ready_payload, command_comment_request, gate)
  local ready_version = M.ready_split_version(state.version)
  local raised = { "github-proxy.github_issue_comment_request", "devloop_ready" }
  local has_blocked_label = M.has_label(current.labels, M._blocked_on_dependency_label)
  local release_fact = M.dependency_release_fact(current.comments, proposal_id, state.version)
  if release_fact == nil then table.insert(raised, "github-proxy.github_issue_comment_request") end
  if command_comment_request ~= nil then table.insert(raised, "github-proxy.github_issue_comment_request") end
  if has_blocked_label then table.insert(raised, "github-proxy.github_issue_label_request") end
  M.log_apply(dept, proposal_id, "ready", ready_version, { add = {}, remove = { M._blocked_on_dependency_label } }, raised)
  M.log_raise(dept, proposal_id, "github-proxy.github_issue_comment_request", M.build_ready_split_canonicalized_comment_request(
    issue.repo, issue.number, proposal_id, state.version, "ready", ready_version, gate, issue.source_ref
  ))
  if command_comment_request ~= nil then
    M.log_raise(dept, proposal_id, "github-proxy.github_issue_comment_request", command_comment_request)
  end
  if release_fact == nil then
    M.log_raise(dept, proposal_id, "github-proxy.github_issue_comment_request", M.build_dependency_release_comment_request(
      issue.repo, issue.number, proposal_id, state.version, gate, issue.source_ref
    ))
  end
  if has_blocked_label then
    M.log_raise(dept, proposal_id, "github-proxy.github_issue_label_request", M.build_label_request(
      issue.repo, issue.number, {}, { M._blocked_on_dependency_label },
      M._dedup_key({ "dependency", "label", "clear", tostring(proposal_id), tostring(state.version) }), issue.source_ref
    ))
  end
  ready_payload.dedup_key = M.build_devloop_ready_payload({
    proposal_id = proposal_id,
    dedup_key = ready_version,
    source_ref = issue.source_ref,
  }).dedup_key
  M.log_raise(dept, proposal_id, "devloop_ready", ready_payload)
  return true
end

local function raise_dependency_wait_hold(M, dept, issue, proposal_id, state, current, gate, command)
  local dependency_hold = M.dependency_hold_fact(current.comments, proposal_id)
  local marker = gate.kind == "cycle"
    and M.dependency_cycle_marker(proposal_id, state.version)
    or (gate.kind == "unresolvable"
      and M.dependency_unresolvable_marker(proposal_id, state.version, gate.unmet, gate.kind, gate.reason)
      or M.dependency_wait_marker(proposal_id, state.version, gate.unmet, gate.kind, gate.reason))
  M.log_cas_decision(dept, proposal_id, state, "dependency_wait", "dependency_wait", "retry-pending(dependency-hold)", gate.reason)
  local raised = {}
  if dependency_hold == nil then
    table.insert(raised, "github-proxy.github_issue_comment_request")
    table.insert(raised, "github-proxy.github_issue_label_request")
  end
  local command_comment_request = nil
  if command ~= nil then
    command_comment_request = M.build_operator_issue_reready_comment_request(issue.repo, issue.number, command, "dependency-hold", issue.source_ref)
    table.insert(raised, "github-proxy.github_issue_comment_request")
  end
  if #raised > 0 then
    M.log_apply(dept, proposal_id, "dependency_wait", state.version, { add = { M._blocked_on_dependency_label }, remove = {} }, raised)
  end
  if command_comment_request ~= nil then
    M.log_raise(dept, proposal_id, "github-proxy.github_issue_comment_request", command_comment_request)
  end
  if dependency_hold == nil then
    M.log_raise(dept, proposal_id, "github-proxy.github_issue_comment_request", M.build_dependency_hold_comment_request(issue.repo, issue.number, proposal_id, state.version, gate, marker, issue.source_ref))
    M.log_raise(dept, proposal_id, "github-proxy.github_issue_label_request", M.build_label_request(
      issue.repo, issue.number, { M._blocked_on_dependency_label }, {},
      M._dedup_key({ "dependency", "label", "hold", tostring(proposal_id), tostring(state.version), tostring(gate.kind) }), issue.source_ref
    ))
  end
  return #raised > 0
end

function M.replay_dependency_wait_state(dept, issue, state, row, facts)
  local proposal_id = facts.proposal_id
  local fields = replay_fields(M, row, state, issue, proposal_id)
  local ready_payload = facts.ready_payload or M.build_devloop_ready_payload({
    proposal_id = fields.proposal_id,
    dedup_key = fields.dedup_key,
    source_ref = fields.source_ref,
  })
  local gate = facts.dependency_gate or M.dependency_gate(issue.repo, issue.number, {
    proposal_id = proposal_id,
    version = state.version,
    comments = facts.current.comments,
  })
  if not gate.ok then
    return raise_dependency_wait_hold(M, dept, issue, proposal_id, state, facts.current, gate, facts.command)
  end
  M.log_cas_decision(dept, proposal_id, state, "dependency_wait", "ready", "release-dependency-hold", gate.reason)
  local command_comment_request = facts.command_comment_request or (facts.command ~= nil
    and M.build_operator_issue_reready_comment_request(issue.repo, issue.number, facts.command, "dependency-release", issue.source_ref)
    or nil)
  return raise_dependency_release(M, dept, issue, proposal_id, state, facts.current, ready_payload, command_comment_request, gate)
end

function M.replay_ready_state(dept, issue, state, row, facts)
  local proposal_id = facts.proposal_id
  local fields = replay_fields(M, row, state, issue, proposal_id)
  local gate = facts.dependency_gate or M.dependency_gate(issue.repo, issue.number, {
    proposal_id = proposal_id,
    version = state.version,
    comments = facts.current.comments,
  })
  if not gate.ok then
    local dep_version = M.ready_split_version(state.version)
    M.log_cas_decision(dept, proposal_id, state, "ready", "dependency_wait", "hold-dependency-reappeared", gate.reason)
    M.log_apply(dept, proposal_id, "dependency_wait", dep_version, { add = { M._blocked_on_dependency_label }, remove = {} }, {
      "github-proxy.github_issue_comment_request",
      "github-proxy.github_issue_label_request",
    })
    M.log_raise(dept, proposal_id, "github-proxy.github_issue_comment_request", M.build_ready_split_canonicalized_comment_request(
      issue.repo, issue.number, proposal_id, state.version, "dependency_wait", dep_version, gate, issue.source_ref
    ))
    M.log_raise(dept, proposal_id, "github-proxy.github_issue_label_request", M.build_label_request(
      issue.repo, issue.number, { M._blocked_on_dependency_label }, {},
      M._dedup_key({ "dependency", "label", "hold", tostring(proposal_id), tostring(dep_version), tostring(gate.kind) }), issue.source_ref
    ))
    return true
  end
  local ready_payload = facts.ready_payload or M.build_devloop_ready_payload({
    proposal_id = fields.proposal_id,
    dedup_key = fields.dedup_key,
    source_ref = fields.source_ref,
  })
  local raised = { "devloop_ready" }
  local command_comment_request = nil
  if facts.command ~= nil then
    command_comment_request = M.build_operator_issue_reready_comment_request(issue.repo, issue.number, facts.command, "ready", issue.source_ref)
    table.insert(raised, "github-proxy.github_issue_comment_request")
  end
  M.log_cas_decision(dept, proposal_id, state, "ready", "implementing", "applied(replay)", "dependency gate is satisfied")
  M.log_apply(dept, proposal_id, nil, nil, { add = {}, remove = {} }, raised)
  if command_comment_request ~= nil then
    M.log_raise(dept, proposal_id, "github-proxy.github_issue_comment_request", command_comment_request)
  end
  M.log_raise(dept, proposal_id, "devloop_ready", ready_payload)
  return true
end

end

return S
