local S = {}

function S.install(M)
local function has_devloop_state_label(labels)
  for _, label in ipairs(labels or {}) do
    if M.is_state_label(label) then
      return true
    end
  end
  return false
end

function M.should_skip_known_intake_issue(labels)
  return M.is_intake_held(labels) or M.is_opted_in(labels) or has_devloop_state_label(labels)
end

function M.pending_reintake_command(comments)
  local command = M.operator_command_fact(comments, "reintake")
  if command ~= nil and not M.has_operator_command_response(comments, command) then
    return command
  end
  return nil
end

function M.intake_candidate_updated_at(issue, command)
  if command ~= nil then
    return command.created_at or issue.updated_at
  end
  return issue.updated_at
end

function M.build_intake_scan_candidate(repo, issue, command, delivery_version)
  local updated_at = M.intake_candidate_updated_at(issue, command)
  local proposal_id = M.proposal_id(repo, tostring(issue.number))
  local effect_id = M.intake_decision_dedup_key(proposal_id, {
    title = issue.title,
    body = issue.body,
  }, command)
  return M.build_devloop_intake_candidate_payload(repo, tostring(issue.number), updated_at, {
    effect_id = effect_id,
    delivery_version = delivery_version,
    reintake_command_created_at = command and command.created_at or nil,
  })
end

function M.read_intake_repo()
  local repo = M.devloop_config().repo
  if repo == nil or not M.issue_ref_round_trips(repo, 1) then
    return nil
  end
  return repo
end
end

return S
