return function(M, h)
  local fact = h.fact
  local obligation = h.obligation
  local effect = h.effect
  local budget = h.budget
  local timeout = h.timeout
  local liveness = h.liveness
  return {
    from_state = "review-meta",
    terminal = false,
    to_states = { "fixing", "blocked" },
    driving_queue = "devloop_review_meta",
    output_obligation = obligation({ "review-meta:v1", "state:v1 fixing", "state:v1 blocked" }, { "fixing", "blocked" }),
    budget = budget(90, "The review-meta receiver is bounded by a 60 minute codex decision attempt plus the standard 30 minute watchdog margin."),
    liveness_contract = liveness({
      mode = "row-budget-bounds-receiver",
      receiver_bound_minutes = 60,
    }),
    on_timeout = timeout("devloop_review_meta"),
    payload_builder = M.build_devloop_review_meta_payload,
    dedup_shape = "review-meta/<proposal_id>/<version>/<pr>/<n>/<review_dedup>",
    required_facts = {
      fact("state", "marker-read"),
      fact("pr-link", "marker-read"),
      fact("review-meta", "marker-read"),
      fact("fix-reflection", "marker-read"),
      fact("review-result", "marker-read"),
      fact("review-converge-round", "marker-read"),
      fact("pr-head", "fetch-before-compare"),
    },
    payload_fields = {
      proposal_id = "marker:review-meta.proposal",
      review_proposal_id = "marker:review-converge-round.proposal",
      review_dedup_key = "marker:review-converge-round.dedup",
      version = "marker:state.version",
      pr_number = "marker:pr-link.pr",
      n = "marker:review-converge-round.round",
      blocking_gap = "marker:review-result.gap",
      source_ref = "source_ref:pr",
    },
    version_identity = "strip_transition_version_suffixes(state.version)",
    effects = effect({ "devloop_review_meta" }, "review-meta replay is complete when review proposal, dedup, PR number, and issue version are reconstructed"),
    marker_facts = "state:v1 review-meta plus review proposal encoded in version/dedup",
    kickoff = "devloop_review_meta",
    replay = "Observe re-raises review-meta using the review proposal, PR number, issue version, and original dedup.",
  }
end
