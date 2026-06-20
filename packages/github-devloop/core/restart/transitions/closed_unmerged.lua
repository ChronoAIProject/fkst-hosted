return function(M, h)
  local responsibility_signature = h.responsibility_signature
  return {
    from_state = "closed-unmerged",
    terminal = true,
    to_states = {},
    responsibility_signature = responsibility_signature({
      receiver_kind = "none",
      driving_queue = "none",
      state_kind = "terminal_hold",
      liveness_class = "terminal",
      input_fact_family = "closed-unmerged-fact",
      output_postcondition_family = "terminal-closed-unmerged",
      phase_rank = M.stage_rank("closed-unmerged"),
      lineage_keys = { "state.version", "pr-origin.pr" },
      successors = {},
    }),
    marker_facts = "state:v1 closed-unmerged",
    replay = "Closed-unmerged is a legal PR terminal state and has no output obligation.",
  }
end
