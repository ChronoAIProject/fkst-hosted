return function(M, h)
  local fact = h.fact
  local obligation = h.obligation
  local effect = h.effect
  local budget = h.budget
  local timeout = h.timeout
  return {
    from_state = "impl-failed",
    terminal = false,
    to_states = { "implementing" },
    driving_queue = "devloop_ready",
    output_obligation = obligation({ "operator reready/reimplement command", "state:v1 implementing" }, { "implementing", "impl-failed" }),
    reentry_commands = { "reready", "reimplement" },
    budget = budget(1440),
    on_timeout = timeout("devloop_ready"),
    payload_builder = M.build_devloop_ready_payload,
    dedup_shape = "ready/<impl-failure inner dedup> with impl_retry_attempt=<impl-failure.attempt+1>",
    required_facts = { fact("state", "marker-read"), fact("impl-failure", "marker-read"), fact("dependency-release", "marker-read") },
    payload_fields = {
      proposal_id = "marker:state.proposal",
      dedup_key = "marker:impl-failure.dedup",
      source_ref = "source_ref:issue",
    },
    version_identity = "ready_payload_inner_version(impl-failure.dedup) plus next_impl_retry_attempt(impl-failure)",
    effects = effect({ "devloop_ready" }, "impl-failed replay is complete when trusted retryable impl-failure attempt is below the retry ceiling"),
    marker_facts = "state:v1 impl-failed plus impl-failure:v1 retryable reason attempt<N",
    kickoff = "devloop_ready",
    replay = "Observe re-raises ready/<version> after one observe tick for bounded retryable implementation failures.",
  }
end
