local h = require("tests.devloop_core_helpers")
local core = h.core
local t = h.t

local function copy_value(value)
  if type(value) ~= "table" then
    return value
  end
  local out = {}
  for key, nested in pairs(value) do
    out[key] = copy_value(nested)
  end
  return out
end

local function rows_by_state(rows)
  local by_state = {}
  for _, row in ipairs(rows or {}) do
    by_state[row.from_state] = row
  end
  return by_state
end

return {
  test_merge_ready_is_approval_wait_handoff_with_explicit_merge_gate_boundary = function()
    local row = rows_by_state(core.restart_transition_table())["merge-ready"]
    local signature = row.responsibility_signature
    local guard = row.guard_boundaries and row.guard_boundaries[1] or nil

    t.eq(row.to_states[1], "merging")
    t.eq(#row.to_states, 2)
    t.eq(row.to_states[2], "blocked")
    t.eq(signature.receiver_kind, "merge-ready-handoff")
    t.eq(signature.state_kind, "queue_wait")
    t.eq(signature.input_fact_family, "head-bound-merge-authorization")
    t.eq(signature.output_postcondition_family, "merge_gate_handoff")
    t.eq(signature.decision_type, nil)
    t.eq(#signature.successors, 2)
    t.eq(signature.successors[1].state, "merging")
    t.eq(signature.successors[1].output_variant, "handoff_to_merge_gate")
    t.eq(signature.successors[1].postcondition_family, "merge_gate_handoff")
    t.eq(signature.successors[2].state, "blocked")
    t.eq(signature.successors[2].terminal, true)

    t.eq(guard.name, "merge_gate")
    t.eq(guard.kind, "guard_table")
    t.eq(guard.gate_kind, "decision")
    t.eq(guard.input_fact_family, "head-bound-merge-authorization")
    t.eq(guard.output_postcondition_family, "merge_eligibility_decided")
    t.eq(guard.decision_type, "MergeEligibility")
    t.eq(#guard.successors, 4)
    t.eq(guard.successors[1].state, "reviewing")
    t.eq(guard.successors[1].output_variant, "approval_stale")
    t.eq(guard.successors[1].decision_type, "MergeEligibility")
    t.eq(guard.successors[2].state, "merging")
    t.eq(guard.successors[2].output_variant, "eligible_now")
    t.eq(guard.successors[2].decision_type, "MergeEligibility")
    t.eq(guard.successors[3].state, "fixing")
    t.eq(guard.successors[3].output_variant, "code_repair_needed")
    t.eq(guard.successors[3].decision_type, "MergeEligibility")
    t.eq(guard.successors[4].state, "blocked")
    t.eq(guard.successors[4].terminal, true)
  end,

  test_merging_is_execution_boundary_not_merge_eligibility_decider = function()
    local row = rows_by_state(core.restart_transition_table()).merging
    local signature = row.responsibility_signature

    t.eq(signature.receiver_kind, "merge-executor")
    t.eq(signature.state_kind, "gate")
    t.eq(signature.gate_kind, "decision")
    t.eq(signature.decision_type, "MergeExecutionResult")
    t.eq(signature.output_postcondition_family, "merge_execution_result")
    for _, edge in ipairs(signature.successors) do
      t.is_true(edge.output_variant ~= "approval_stale")
      t.is_true(edge.output_variant ~= "code_repair_needed")
      t.is_true(edge.decision_type ~= "MergeEligibility")
    end
  end,

  test_merge_origin_fix_budget_escape_is_terminal_not_failure = function()
    local rows = rows_by_state(core.restart_transition_table())
    for _, expected in ipairs({
      { state = "merge-ready", edge = 2 },
      { state = "merging", edge = 4 },
    }) do
      local row = rows[expected.state]
      local edge = row.responsibility_signature.successors[expected.edge]
      t.eq(row.to_states[expected.edge], "blocked")
      t.eq(row.output_obligation.exits[expected.edge], "blocked")
      t.eq(edge.state, "blocked")
      t.eq(edge.output_variant, "fix_budget_exhausted")
      t.eq(edge.failure, nil)
      t.eq(edge.terminal, true)
      t.eq(edge.monotonic, true)
      t.eq(core.state_successors(expected.state)[expected.edge], "blocked")
    end
    t.eq(#core.strict_restart_responsibility_contract_errors(core.restart_transition_table()), 0)
  end,

  test_fixing_code_producer_keys_liveness_to_revision_goal_work_unit = function()
    local row = rows_by_state(core.restart_transition_table()).fixing
    local signature = row.responsibility_signature
    local match = row.liveness_contract.real_execution.match
    local lineage = {}
    for _, key in ipairs(signature.lineage_keys or {}) do
      lineage[key] = true
    end

    t.eq(signature.receiver_kind, "code-producer")
    t.eq(signature.input_fact_family, "revision-goal")
    t.eq(match.dedup_key, "state.work_unit_key")
    t.eq(lineage["revision-goal.work_unit_key"], true)
    t.eq(lineage["ci-failure.ci_failure_key"], nil)
    t.is_true(row.dedup_shape:find("ci-failure:<proposal_id>/<pr>/<version>", 1, true) ~= nil)
    t.is_true(row.dedup_shape:find("<ci_failure_key-or-noci>", 1, true) == nil)
    t.eq(row.payload_fields.work_unit_key, "dedup:fixing-work-unit")
    t.eq(row.payload_fields.ci_failure_key, "marker:merge-gate.ci_failure_key")
    t.eq(row.to_states[3], "blocked")
    t.eq(row.output_obligation.exits[4], "blocked")
    t.eq(signature.successors[3].state, "blocked")
    t.eq(signature.successors[3].output_variant, "fix_budget_exhausted")
    t.eq(signature.successors[3].terminal, true)
    t.eq(core.state_successors("fixing")[3], "blocked")
    -- the fixing->blocked budget-exhaustion escape is terminal (not a second failure family);
    -- the row must have zero strict restart-responsibility contract errors.
    t.eq(#core.strict_restart_responsibility_contract_errors(core.restart_transition_table()), 0)
    t.is_true(table.concat(row.output_obligation.kinds, ","):find("ci-repair-attempt:v1", 1, true) ~= nil)
  end,

  test_fixing_code_producer_rejects_version_keyed_ci_repair_liveness = function()
    local row = copy_value(rows_by_state(core.restart_transition_table()).fixing)
    row.liveness_contract.real_execution.match.dedup_key = "state.version"
    local strict_errors = core.strict_restart_liveness_contract_errors({ row })
    local generic_errors = core.liveness_contract_errors({ row })
    t.is_true(table.concat(strict_errors, "\n"):find("fixing: code-producing CI repair codex_run defer real_execution.match.dedup_key must be state.work_unit_key", 1, true) ~= nil)
    t.is_true(table.concat(generic_errors, "\n"):find("fixing: live-defer code-producing CI repair real_execution.match.dedup_key must be state.work_unit_key", 1, true) ~= nil)
  end,
}
