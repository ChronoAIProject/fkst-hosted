return function(M, h)
  local fact = h.fact
  local obligation = h.obligation
  local effect = h.effect
  local budget = h.budget
  local timeout = h.timeout
  local liveness = h.liveness
  local watchdog = h.watchdog
  local actionable_epoch = h.actionable_epoch
  return {
    from_state = "ready",
    liveness_class_id = "ready.actionable",
    watchdog = watchdog("live-defer", 45),
    actionable_epoch = {
      source = "live_defer_epoch:v1",
      generation_source = "same_as_actionable_epoch",
      allows_state_entry_if_never_deferred = true,
    },
    defer = {
      kind = "release_gate",
      live_marker = "dependency-wait:v1",
      freshness_ms = 525600 * 60 * 1000,
      clear_fact = "dependency-release:v1",
      observed_fact = "dependency-wait-observed:v1",
      clear_opens_generation = true,
    },
    terminal = false,
    to_states = { "implementing" },
    driving_queue = "devloop_ready",
    observe_surfaces = { issue = true, liveness_scan = true },
    output_obligation = obligation({ "state:v1 implementing", "dependency-hold:v1" }, { "implementing", "ready" }),
    budget = budget(45, "Ready is deferred by dependency-wait heartbeats when blocked; otherwise implementation kickoff is expected inside the watchdog margin."),
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
