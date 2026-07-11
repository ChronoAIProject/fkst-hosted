local devloop_state = require("devloop.state")
return function(M, h)
  local responsibility_signature = h.responsibility_signature
  return {
    from_state = "declined",
    terminal = true,
    to_states = {},
    responsibility_signature = responsibility_signature({
      receiver_kind = "none",
      driving_queue = "none",
      state_kind = "terminal_hold",
      liveness_class = "terminal",
      input_fact_family = "premise-refutation",
      output_postcondition_family = "terminal-declined",
      phase_rank = devloop_state.stage_rank("declined"),
      lineage_keys = { "state.version", "result.dedup" },
      successors = {},
    }),
    marker_facts = "state:v1 declined plus result:v1 decision=reject reason=premise-refuted",
    replay = "Declined is a legal terminal state and has no output obligation.",
  }
end
