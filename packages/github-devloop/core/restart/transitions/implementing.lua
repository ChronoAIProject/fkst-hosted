return function(M, h)
  local fact = h.fact
  local obligation = h.obligation
  local effect = h.effect
  local budget = h.budget
  local timeout = h.timeout
  local liveness = h.liveness
  return {
    from_state = "implementing",
    terminal = false,
    to_states = { "pr-open", "impl-failed" },
    driving_queue = "devloop_ready",
    observe_surfaces = { issue = true, liveness_scan = true },
    output_obligation = obligation({ "state:v1 pr-open", "state:v1 impl-failed" }, { "pr-open", "impl-failed" }),
    budget = budget(45, "The long implementation receiver is supervised by implement-attempt heartbeats; this budget only bounds stale heartbeat redrive."),
    liveness_contract = liveness({
      mode = "live-defer",
      signal = {
        family = "implement-attempt",
        producer = "implement-attempt",
        surface = "issue-comment-stream",
        version_form = "raw",
        max_age_minutes = 120,
      },
    }),
    on_timeout = timeout("devloop_ready"),
    payload_builder = M.build_devloop_ready_payload,
    dedup_shape = "ready/<implementing_inner_version> with impl_retry_attempt=<implementation_retry_attempt(state.version)>",
    required_facts = {
      fact("state", "marker-read"),
      fact("implementing", "marker-read"),
      fact("implement-attempt", "marker-read"),
      fact("branch-head", "fetch-before-compare"),
    },
    payload_fields = {
      proposal_id = "marker:state.proposal",
      dedup_key = "marker:state.version",
      source_ref = "source_ref:issue",
    },
    version_identity = "ready_payload_inner_version(state.version) plus implementation_retry_attempt(state.version)",
    effects = effect({ "devloop_ready" }, "implementing replay is complete only when observe_issue can re-raise devloop_ready with the frozen implementing version for implement to re-derive PR link, remote branch, local branch, or bounded retry"),
    marker_facts = "state:v1 implementing plus implementing:v1 and implement-attempt:v1",
    kickoff = "devloop_ready",
    replay = "Observe re-raises devloop_ready only when the implement attempt is past its liveness budget; implement then re-derives PR link, remote branch, local branch, or bounded retry.",
  }
end
