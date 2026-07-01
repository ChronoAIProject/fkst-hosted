local saga_conformance = require("devloop.saga_conformance")
local contract_time = require("contract.time")
local M

-- fkst.toml conformance hook: function = "core.saga_conformance_errors" (delegates to typed devloop.saga_conformance.errors)
local function saga_conformance_errors()
  return saga_conformance.errors(M)
end

M = {
  saga_conformance_errors = saga_conformance_errors,
}


function M.decompose_package_queue()
  return "github-devloop-decompose.devloop_decompose"
end

function M.liveness_state_age_minutes(state, now_seconds)
  if type(state) ~= "table" then
    return nil
  end
  if state.marker_created_at ~= nil and state.marker_created_at ~= "" then
    local created_seconds = contract_time.iso_timestamp_epoch_seconds(state.marker_created_at)
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
require("forge.github_debug_stamp").install(M)
require("devloop.commands").install(M)
require("devloop.github_proxy_entity_view").install(M)
require("devloop.pr_safety").install(M)
require("devloop.git_mechanics").install(M)
require("devloop.claims").install(M)
require("devloop.logging").install(M)
require("devloop.state").install(M)
require("core.error_facts").install(M)
require("core.failure_triage").install(M)
require("core.conflict_telemetry").install(M)
require("core.dependency_wait").install(M)
require("core.state_gap").install(M)
require("devloop.entity").install(M)
require("core.observability_bounds").install(M)
require("core.ensure_repo").install(M)
require("core.doctor").install(M)

return M
