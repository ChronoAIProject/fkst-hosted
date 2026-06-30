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
    from_state = "merging",
    liveness_class_id = "merging.actionable",
    watchdog = watchdog("row-budget-bounds-receiver", 390),
    actionable_epoch = actionable_epoch("state_entry:v1"),
    terminal = false,
    to_states = { "merged", "reviewing", "fixing", "blocked" },
    driving_queue = "devloop_merge_ready",
    observe_surfaces = { issue = true, pr = true, liveness_scan = true },
    output_obligation = obligation({ "merged:v1", "state:v1 reviewing", "state:v1 fixing", "state:v1 blocked" }, { "merged", "reviewing", "fixing", "blocked" }),
    budget = budget(390, "The merging receiver is bounded by 30 minutes of merge work plus a 360 minute external CI wait window."),
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
      receiver_kind = "merge-executor",
      driving_queue = "devloop_merge_ready",
      state_kind = "gate",
      gate_kind = "decision",
      liveness_class = "merging.actionable",
      input_fact_family = "head-bound-merge-authorization",
      output_postcondition_family = "merge_execution_result",
      decision_type = "MergeExecutionResult",
      phase_rank = M.stage_rank("merging"),
      lineage_keys = { "merge-ready.version", "merge-ready.head_sha", "source_ref" },
      successors = {
        {
          state = "merged",
          output_variant = "merge-completed",
          postcondition_family = "merge_execution_result",
          decision_type = "MergeExecutionResult",
          monotonic = true,
        },
        {
          state = "reviewing",
          output_variant = "head-advanced",
          postcondition_family = "merge_execution_result",
          decision_type = "MergeExecutionResult",
          bump = true,
        },
        {
          state = "fixing",
          output_variant = "merge-needs-fix",
          postcondition_family = "merge_execution_result",
          decision_type = "MergeExecutionResult",
          failure = true,
          bump = true,
        },
        {
          state = "blocked",
          output_variant = "merge-blocked",
          postcondition_family = "merge_execution_result",
          decision_type = "MergeExecutionResult",
          failure = true,
          terminal = true,
          monotonic = true,
        },
      },
    }),
    payload_builder = payloads_builders.build_devloop_merge_ready_payload,
    dedup_shape = "merge-ready/<proposal_id>/<version>/<pr>/<review_dedup>/<current_head>",
    required_facts = {
      fact("state", "marker-read"),
      fact("merge-ready", "marker-read"),
      fact("merging", "marker-read"),
      fact("review-result", "marker-read"),
      fact("merge-gate-wait", "marker-read"),
      fact("pr-head", "fetch-before-compare"),
      fact("ci-status", "fetch-before-compare"),
    },
    advancing_facts = {
      advancing_fact("merging", "merged", { pr = true, liveness_scan = true }, "source_ref:pr"),
      advancing_fact("merging", "reviewing", { pr = true, liveness_scan = true }, "source_ref:pr"),
      advancing_fact("merging", "fixing", { pr = true, liveness_scan = true }, "source_ref:pr"),
      advancing_fact("merging", "blocked", { pr = true, liveness_scan = true }, "source_ref:pr"),
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
      { "devloop_merge_ready", "pr-state-label" },
      "merging retry is complete when merge-ready and merging markers bind the same fetched PR head and the PR-local state label projection is requested",
      "build_reconcile_pr_state_label_request"
    ),
    marker_facts = "state:v1 merging plus merging:v1",
    kickoff = "devloop_merge_ready",
    replay = "Merge retry re-derives completion or repair from PR mergeability and head facts.",
  }
end
