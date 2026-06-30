local payloads_builders = require("devloop.payloads.builders")
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
  local advancing_fact = h.advancing_fact
  return {
    from_state = "merge-ready",
    liveness_class_id = "merge_ready.actionable",
    watchdog = watchdog("row-budget-bounds-receiver", 390),
    actionable_epoch = actionable_epoch("state_entry:v1"),
    terminal = false,
    to_states = { "merging", "blocked" },
    driving_queue = "devloop_merge_ready",
    observe_surfaces = { issue = true, pr = true, liveness_scan = true },
    output_obligation = obligation({ "state:v1 merging", "state:v1 blocked" }, { "merging", "blocked" }),
    budget = budget(390, "The merge-ready receiver is bounded by 30 minutes of merge work plus a 360 minute external CI wait window."),
    liveness_contract = liveness({
      mode = "row-budget-bounds-receiver",
      receiver_bound_minutes = 30,
      external_wait_bound_minutes = 360,
      progress_signal = {
        family = "merge-gate-wait",
        producer = "merge-gate-wait",
        resolver = "merge-gate-wait",
        surface = "pr-comment-stream",
        version_form = "raw",
        max_age_minutes = 360,
      },
    }),
    on_timeout = timeout("devloop_merge_ready"),
    responsibility_signature = responsibility_signature({
      receiver_kind = "merge-ready-handoff",
      driving_queue = "devloop_merge_ready",
      state_kind = "queue_wait",
      liveness_class = "merge_ready.actionable",
      input_fact_family = "head-bound-merge-authorization",
      output_postcondition_family = "merge_gate_handoff",
      phase_rank = M.stage_rank("merge-ready"),
      lineage_keys = { "merge-ready.version", "merge-ready.pr", "merge-ready.head_sha", "merge-ready.review_dedup", "source_ref" },
      successors = {
        {
          state = "merging",
          output_variant = "handoff_to_merge_gate",
          postcondition_family = "merge_gate_handoff",
          monotonic = true,
        },
        {
          state = "blocked",
          output_variant = "watchdog_reconcile_terminal",
          terminal = true,
          monotonic = true,
        },
      },
    }),
    guard_boundaries = {
      {
        name = "merge_gate",
        kind = "guard_table",
        gate_kind = "decision",
        input_fact_family = "head-bound-merge-authorization",
        output_postcondition_family = "merge_eligibility_decided",
        decision_type = "MergeEligibility",
        successors = {
          {
            state = "reviewing",
            output_variant = "approval_stale",
            decision_type = "MergeEligibility",
            bump = true,
          },
          {
            state = "merging",
            output_variant = "eligible_now",
            decision_type = "MergeEligibility",
            monotonic = true,
          },
          {
            state = "fixing",
            output_variant = "code_repair_needed",
            decision_type = "MergeEligibility",
            failure = true,
            bump = true,
          },
          {
            state = "blocked",
            output_variant = "watchdog_reconcile_terminal",
            failure = true,
            terminal = true,
            monotonic = true,
          },
        },
      },
    },
    payload_builder = payloads_builders.build_devloop_merge_ready_payload,
    dedup_shape = "merge-ready/<proposal_id>/<version>/<pr>/<review_dedup>/<current_head>",
    required_facts = {
      fact("state", "marker-read"),
      fact("pr-link", "marker-read"),
      fact("review-result", "marker-read"),
      fact("merge-ready", "marker-read"),
      fact("review-carry-over", "marker-read"),
      fact("merge-gate-wait", "marker-read"),
      fact("pr-head", "fetch-before-compare"),
      fact("base-head", "fetch-before-compare"),
    },
    advancing_facts = {
      advancing_fact("merge-ready", "merging", { pr = true, liveness_scan = true }, "source_ref:pr"),
      advancing_fact("merge-ready", "blocked", { pr = true, liveness_scan = true }, "source_ref:pr"),
    },
    payload_fields = {
      proposal_id = "marker:merge-ready.proposal",
      pr_number = "marker:merge-ready.pr",
      version = "marker:merge-ready.version",
      review_proposal_id = "marker:merge-ready.review_proposal",
      review_dedup_key = "marker:merge-ready.review_dedup",
      reviewed_head_sha = "marker:merge-ready.head_sha",
      source_ref = "source_ref:pr",
    },
    version_identity = "strip_transition_version_suffixes(merge-ready.version)",
    effects = effect(
      { "review-carry-over-marker", "devloop_merge_ready", "pr-state-label" },
      "merge-ready replay is complete when head-bound approval and fetched PR head match, or when review_carry_over_marker proves the carried approval marker was written; the PR-local state label projection is requested when the PR label is stale",
      "review_carry_over_marker"
    ),
    marker_facts = "state:v1 merge-ready plus merge-ready:v1",
    kickoff = "devloop_merge_ready",
    replay = "PR observe or merge retry re-derives merge-ready from head-bound approval facts.",
  }
end
