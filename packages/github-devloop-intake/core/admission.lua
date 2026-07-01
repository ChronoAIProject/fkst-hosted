local devloop_base = require("devloop.base")
local base_ids = require("devloop.base_ids")
local payloads_builders = require("devloop.payloads.builders")
local S = {}
local operator_commands = require("devloop.operator_commands")

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
  return devloop_base.is_intake_held(labels) or devloop_base.is_opted_in(labels) or has_devloop_state_label(labels)
end

function M.pending_reintake_command(comments)
  local command = operator_commands.operator_command_fact(M, comments, "reintake")
  if command ~= nil and not operator_commands.has_operator_command_response(M, comments, command) then
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

function M.build_intake_admission_candidate(repo, issue, command, delivery_version)
  local updated_at = M.intake_candidate_updated_at(issue, command)
  local proposal_id = base_ids.proposal_id(repo, tostring(issue.number))
  local effect_id = devloop_base.intake_decision_dedup_key(proposal_id, {
    title = issue.title,
    body = issue.body,
  }, command)
  return payloads_builders.build_devloop_intake_candidate_payload(M, repo, tostring(issue.number), updated_at, {
    effect_id = effect_id,
    delivery_version = delivery_version,
    reintake_command_created_at = command and command.created_at or nil,
  })
end
end

return S
