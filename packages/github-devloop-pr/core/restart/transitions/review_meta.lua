return function(M, h)
  local fact = h.fact
  local obligation = h.obligation
  local effect = h.effect
  local budget = h.budget
  local timeout = h.timeout
  local liveness = h.liveness
  local watchdog = h.watchdog
  local actionable_epoch = h.actionable_epoch
  local responsibility_signature = h.responsibility_signature
  return {
    from_state = "review-meta",
    liveness_class_id = "review_meta.actionable",
    watchdog = watchdog("row-budget-bounds-receiver", 90),
    actionable_epoch = actionable_epoch("state_entry:v1"),
    terminal = false,
    to_states = { "fixing", "blocked" },
    driving_queue = "devloop_review_meta",
    observe_surfaces = { issue = true, pr = true, liveness_scan = true },
    output_obligation = obligation({ "review-meta:v1", "state:v1 fixing", "state:v1 blocked" }, { "fixing", "blocked" }),
    budget = budget(90, "The review-meta receiver is bounded by a 60 minute codex decision attempt plus the standard 30 minute watchdog margin."),
    liveness_contract = liveness({
      mode = "row-budget-bounds-receiver",
      receiver_bound_minutes = 60,
    }),
    on_timeout = timeout("devloop_review_meta"),
    responsibility_signature = responsibility_signature({
      receiver_kind = "review-meta-judge",
      driving_queue = "devloop_review_meta",
      state_kind = "decision",
      liveness_class = "review_meta.actionable",
      input_fact_family = "review-convergence-gap",
      output_postcondition_family = "review-meta-decision",
      decision_type = "review-meta-decision",
      phase_rank = M.stage_rank("review-meta"),
      lineage_keys = { "state.version", "review-converge-round.proposal", "review-converge-round.dedup", "source_ref" },
      successors = {
        {
          state = "fixing",
          output_variant = "fix",
          postcondition_family = "review-meta-decision",
          decision_type = "review-meta-decision",
          bump = true,
        },
        {
          state = "blocked",
          output_variant = "block",
          postcondition_family = "review-meta-decision",
          decision_type = "review-meta-decision",
          monotonic = true,
        },
      },
    }),
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
