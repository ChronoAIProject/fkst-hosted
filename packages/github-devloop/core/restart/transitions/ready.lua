return function(M, h)
  local fact = h.fact
  local obligation = h.obligation
  local effect = h.effect
  local budget = h.budget
  local timeout = h.timeout
  local liveness = h.liveness
  return {
    from_state = "ready",
    terminal = false,
    to_states = { "implementing" },
    driving_queue = "devloop_ready",
    output_obligation = obligation({ "state:v1 implementing", "dependency-hold:v1" }, { "implementing", "ready" }),
    budget = budget(45),
    liveness_contract = liveness({
      mode = "live-defer",
      signal = {
        family = "dependency-wait",
        resolver = "dependency-hold",
        producer = "dependency-wait",
        surface = "issue-comment-stream",
        version_form = "raw",
        max_age_minutes = 525600,
      },
    }),
    on_timeout = timeout("devloop_ready"),
    payload_builder = M.build_devloop_ready_payload,
    dedup_shape = "ready/<state.version>",
    required_facts = { fact("state", "marker-read"), fact("dependency-release", "marker-read") },
    payload_fields = {
      proposal_id = "marker:state.proposal",
      dedup_key = "marker:state.version",
      source_ref = "source_ref:issue",
    },
    version_identity = "strip_transition_version_suffixes(state.version)",
    effects = effect(
      { "result-marker", "ready-label", "devloop_ready" },
      "ready replay is complete only when the result marker and ready label are visible, and observe_issue can re-raise devloop_ready while still ready",
      "result_effects_complete"
    ),
    marker_facts = "state:v1 ready",
    kickoff = "devloop_ready",
    replay = "Raise ready/<version> after dependency gate re-derives satisfied blockers.",
  }
end
