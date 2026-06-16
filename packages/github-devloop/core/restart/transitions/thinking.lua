return function(M, h)
  local fact = h.fact
  local obligation = h.obligation
  local effect = h.effect
  local budget = h.budget
  local timeout = h.timeout
  local liveness = h.liveness
  return {
    from_state = "thinking",
    terminal = false,
    to_states = { "ready", "blocked" },
    driving_queue = "consensus.proposal",
    output_obligation = obligation({ "consensus.consensus_reached", "consensus.consensus_converge" }, { "ready", "blocked", "thinking" }),
    budget = budget(150),
    liveness_contract = liveness({
      mode = "live-defer",
      signal = {
        family = "converge-round",
        producer = "converge-round",
        surface = "issue-comment-stream",
        version_form = "raw",
        max_age_minutes = 120,
      },
    }),
    on_timeout = timeout("consensus.proposal"),
    payload_builder = M.build_proposal,
    dedup_shape = "proposal:<proposal_id>/<updated_at> or consensus:<base_version>/loop/<n>",
    required_facts = { fact("state", "marker-read") },
    payload_fields = {
      proposal_id = "marker:state.proposal",
      dedup_key = "marker:state.version",
      source_ref = "source_ref:issue",
    },
    version_identity = "strip_transition_version_suffixes(state.version)",
    effects = effect({ "consensus.proposal" }, "consensus proposal dedup is derived from state.version or next complete converge-round"),
    marker_facts = "state:v1 thinking plus optional converge-round:v1",
    kickoff = "consensus.proposal",
    replay = "Initial thinking reuses the state version as proposal dedup; convergence replays the next /loop/N from the latest complete converge-round marker.",
  }
end
