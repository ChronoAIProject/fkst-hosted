local devloop_base = require("devloop.base")
local base_ids = require("devloop.base_ids")
local payloads_builders = require("devloop.payloads.builders")
local S = {}
local operator_commands = require("devloop.operator_commands")

function S.install(M)
function M.should_skip_known_intake_issue(labels)
  return devloop_base.is_intake_held(labels)
    or devloop_base.is_opted_in(labels)
    or operator_commands.reintake_has_active_devloop_state(labels, nil, nil)
end

function M.reintake_has_active_devloop_state(labels, comments, proposal_id)
  return operator_commands.reintake_has_active_devloop_state(labels, comments, proposal_id)
end

function M.pending_reintake_command(comments)
  local command = operator_commands.operator_command_fact(comments, "reintake")
  if command ~= nil and not operator_commands.has_operator_command_response(comments, command) then
    return command
  end
  return nil
end

function M.intake_candidate_updated_at(issue, command, comments, proposal_id)
  if command ~= nil then
    return operator_commands.reintake_effect_updated_at(issue, command, comments, proposal_id)
  end
  return issue.updated_at
end

function M.build_intake_admission_candidate(repo, issue, command, delivery_version, comments)
  local proposal_id = base_ids.proposal_id(repo, tostring(issue.number))
  local updated_at = M.intake_candidate_updated_at(issue, command, comments, proposal_id)
  local effect_id = devloop_base.intake_decision_dedup_key(proposal_id, {
    title = issue.title,
    body = issue.body,
  }, command, command and updated_at or nil)
  return payloads_builders.build_devloop_intake_candidate_payload(repo, tostring(issue.number), updated_at, {
    effect_id = effect_id,
    delivery_version = delivery_version,
    reintake_command_created_at = command and command.created_at or nil,
    reintake_effect_updated_at = command and updated_at or nil,
  })
end
end

return S
