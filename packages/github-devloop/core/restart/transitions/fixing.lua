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
    from_state = "fixing",
    liveness_class_id = "fixing.actionable",
    watchdog = watchdog("row-budget-bounds-receiver", 120),
    actionable_epoch = actionable_epoch("state_entry:v1"),
    terminal = false,
    to_states = { "reviewing" },
    driving_queue = "devloop_fixing",
    observe_surfaces = { issue = true, pr = true, liveness_scan = true },
    output_obligation = obligation({ "fix:v1", "state:v1 reviewing" }, { "reviewing", "fixing" }),
    budget = budget(120, "The fixing receiver is bounded by a 60 minute codex repair attempt plus the standard 30 minute watchdog margin and retry slack."),
    liveness_contract = liveness({
      mode = "row-budget-bounds-receiver",
      receiver_bound_minutes = 60,
    }),
    on_timeout = timeout("devloop_fixing"),
    responsibility_signature = responsibility_signature({
      receiver_kind = "code-producer",
      driving_queue = "devloop_fixing",
      state_kind = "worker",
      liveness_class = "fixing.actionable",
      input_fact_family = "fix-feedback",
      output_postcondition_family = "revision_published",
      phase_rank = M.stage_rank("fixing"),
      lineage_keys = { "state.version", "review-result.dedup", "review-result.head_sha", "source_ref" },
      successors = {
        {
          state = "reviewing",
          output_variant = "revision_published",
          postcondition_family = "revision_published",
          bump = true,
        },
      },
    }),
    payload_builder = M.build_devloop_fixing_payload,
    dedup_shape = "forward:fixing/<proposal_id>/<version>/<pr>/<review_dedup>; replay:fixing/replay/<proposal_id>/<version>/<pr>/<review_dedup>/<gate_baseline_sha-or-nobase>/<reviewed_head_sha>",
    required_facts = {
      fact("state", "marker-read"),
      fact("pr-link", "marker-read"),
      fact("review-result", "marker-read"),
      fact("review-meta", "marker-read"),
      fact("merge-gate", "marker-read"),
      fact("pr-head", "fetch-before-compare"),
    },
    payload_fields = {
      schema = "literal:github-devloop.fixing.v1",
      proposal_id = "marker:state.proposal",
      pr_number = "marker:pr-link.pr",
      version = "marker:state.version",
      review_proposal_id = "marker:merge-gate.review_proposal",
      review_dedup_key = "marker:merge-gate.review_dedup",
      reviewed_head_sha = "marker:merge-gate.head_sha",
      dedup_key = "dedup:replayed-fixing",
      gate_baseline_sha = "marker:merge-gate.gate_baseline_sha",
      gate_failure_excerpt = "comment_body:fix-feedback",
      source_ref = "source_ref:pr",
    },
    version_identity = "strip_transition_version_suffixes(state.version)",
    effects = effect(
      { "devloop_fixing", "pr-state-label" },
      "fixing replay is complete only when trusted feedback marker fields are copied into devloop_fixing and the PR-local state label projection is requested",
      "build_reconcile_pr_state_label_request"
    ),
    marker_facts = "state:v1 fixing plus review-result/review-meta/merge-gate feedback, or current PR head for deterministic renormalization",
    kickoff = "devloop_fixing or devloop_reviewing",
    replay = "Observe re-raises fix when a trusted feedback fact is parseable; otherwise it re-enters reviewing for the current head.",
  }
end
