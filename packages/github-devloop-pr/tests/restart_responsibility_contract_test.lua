local h = require("tests.devloop_core_helpers")
local core = h.core
local t = h.t

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
}
