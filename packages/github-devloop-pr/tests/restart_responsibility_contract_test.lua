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
  test_merge_ready_is_approval_wait_handoff_not_merge_eligibility_decider = function()
    local row = rows_by_state(core.restart_transition_table())["merge-ready"]
    local signature = row.responsibility_signature

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
  end,
}
