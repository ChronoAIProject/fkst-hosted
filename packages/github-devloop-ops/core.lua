local M = {}

function M.persistence_class()
  return "saga"
end

local lifecycle_rows = {
  ["awaiting-pr"] = { from_state = "awaiting-pr", terminal = false, driving_queue = "devloop_observe_redrive", budget = { minutes = 180 * 24 * 60 } },
  blocked = { from_state = "blocked", terminal = false, driving_queue = "github-devloop-decompose.devloop_decompose", budget = { minutes = 1440 } },
  dependency_wait = { from_state = "dependency_wait", terminal = false, driving_queue = "devloop_observe_redrive", budget = { minutes = 525600 } },
  ["impl-failed"] = { from_state = "impl-failed", terminal = false, driving_queue = "devloop_ready", budget = { minutes = 1440 } },
  implementing = { from_state = "implementing", terminal = false, driving_queue = "devloop_ready", budget = { minutes = 120 } },
  merged = { from_state = "merged", terminal = true, driving_queue = "none", budget = nil },
  ready = { from_state = "ready", terminal = false, driving_queue = "devloop_ready", budget = { minutes = 120 } },
  thinking = { from_state = "thinking", terminal = false, driving_queue = "consensus.proposal", budget = { minutes = 150 } },
}

function M.lifecycle_transition_row(state_name)
  return lifecycle_rows[state_name]
end

function M.liveness_budget_minutes(state_name)
  local row = M.lifecycle_transition_row(state_name)
  return row and row.budget and tonumber(row.budget.minutes) or nil
end

function M.liveness_state_age_minutes(state, now_seconds)
  if type(state) ~= "table" then
    return nil
  end
  if state.marker_created_at ~= nil and state.marker_created_at ~= "" then
    local created_seconds = M.iso_timestamp_epoch_seconds(state.marker_created_at)
    local current_seconds = tonumber(now_seconds)
    if created_seconds ~= nil and current_seconds ~= nil and current_seconds >= created_seconds then
      return math.floor((current_seconds - created_seconds) / 60)
    end
  end
  return M.stall_suspect_age_minutes(state.version, now_seconds)
end

function M.liveness_timeout_due(row, state, now_seconds)
  if row == nil or row.terminal == true then
    return false, nil
  end
  local budget = row.budget and tonumber(row.budget.minutes) or nil
  local age = M.liveness_state_age_minutes(state, now_seconds)
  if budget == nil or age == nil or age < budget then
    return false, age
  end
  return true, age
end

require("devloop.base").install(M)
require("devloop.config").install(M)
require("forge.github_debug_stamp").install(M)
require("devloop.strings").install(M)
require("devloop.commands").install(M)
require("devloop.entity_list_cache").install(M)
require("devloop.github_proxy_entity_view").install(M)
require("devloop.github_risk").install(M)
require("devloop.pr_safety").install(M)
require("devloop.queue").install(M)
require("devloop.merge_queue").install(M)
require("devloop.queue_starvation").install(M)
require("devloop.git_mechanics").install(M)
require("devloop.claims").install(M)
require("devloop.parsers").install(M)
require("devloop.autonomy_ledger").install(M)
require("devloop.logging").install(M)
require("devloop.conflict_telemetry").install(M)
require("core.error_facts").install(M)
require("core.failure_triage").install(M)
require("core.conflict_telemetry").install(M)
require("devloop.state").install(M)
require("core.dependency_wait").install(M)
require("core.state_gap").install(M)
require("devloop.markers").install(M)
require("devloop.payloads").install(M)
require("devloop.decompose").install(M)
require("devloop.requests").install(M)
require("devloop.entity").install(M)
require("devloop.sweep_bounds").install(M)
require("core.observability_bounds").install(M)
require("core.observability").install(M)
require("core.ensure_repo").install(M)
require("devloop.context_bundle").install(M)
require("devloop.operator_commands").install(M)
require("core.doctor").install(M)

return M
