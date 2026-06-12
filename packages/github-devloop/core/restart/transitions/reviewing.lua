return function(M, h)
  local fact = h.fact
  local obligation = h.obligation
  local effect = h.effect
  local budget = h.budget
  local timeout = h.timeout
  return {
    from_state = "reviewing",
    terminal = false,
    to_states = { "merge-ready", "fixing", "review-meta", "blocked" },
    driving_queue = "devloop_reviewing",
    output_obligation = obligation({ "review-result:v1", "review-converge-round:v1", "state:v1 blocked" }, { "merge-ready", "fixing", "review-meta", "blocked", "reviewing" }),
    budget = budget(120),
    on_timeout = timeout("devloop_reviewing"),
    payload_builder = M.build_devloop_reviewing_payload,
    dedup_shape = "reviewing/<proposal_id>/<state.version>/<pr>",
    required_facts = {
      fact("state", "marker-read"),
      fact("pr-link", "marker-read"),
      fact("pr-head", "fetch-before-compare"),
      fact("review-converge-round", "marker-read"),
    },
    payload_fields = {
      proposal_id = "marker:state.proposal",
      version = "marker:state.version",
      pr_number = "marker:pr-link.pr",
      source_ref = "source_ref:pr",
    },
    version_identity = "strip_transition_version_suffixes(state.version)",
    effects = effect({ "devloop_reviewing" }, "review payload is complete when current PR head is fetched and no head-bound review result exists"),
    marker_facts = "state:v1 reviewing plus PR head facts",
    kickoff = "devloop_reviewing",
    replay = "PR observe re-derives review kickoff from current PR head and issue version.",
  }
end
