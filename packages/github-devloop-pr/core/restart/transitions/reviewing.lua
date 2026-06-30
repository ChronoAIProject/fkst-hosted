local payloads_builders = require("devloop.payloads.builders")
return function(M, h)
  local fact = h.fact
  local obligation = h.obligation
  local effect = h.effect
  local budget = h.budget
  local timeout = h.timeout
  local liveness = h.liveness
  local watchdog = h.watchdog
  local responsibility_signature = h.responsibility_signature
  local advancing_fact = h.advancing_fact
  return {
    from_state = "reviewing",
    generation_entry = {
      reentry_bump = true,
      birth_from = "pr-open",
    },
    liveness_class_id = "reviewing.active",
    watchdog = {
      mode = "live-defer",
      budget_ms = 150 * 60 * 1000,
      on_stale = {
        op = "redrive_receiver",
        producer = "review-converge-round",
      },
    },
    actionable_epoch = {
      source = "live_defer_heartbeat:v1",
      generation_source = "same_as_actionable_epoch",
      live_marker = "review-converge-round:v1",
      producer = "review-converge-round",
    },
    defer = {
      kind = "heartbeat",
      live_marker = "review-converge-round:v1",
      producer = "review-converge-round",
      freshness_ms = 120 * 60 * 1000,
      redrive_opens_generation = true,
    },
    terminal = false,
    to_states = { "merge-ready", "fixing", "review-meta", "blocked" },
    driving_queue = "devloop_reviewing",
    observe_surfaces = { issue = true, pr = true, liveness_scan = true },
    timeout_surfaces = { issue = true, pr = true, liveness_scan = true },
    pr_recovery = {
      not_mergeable = {
        to_state = "fixing",
        queue = "devloop_fixing",
      },
    },
    output_obligation = obligation({ "review-result:v1", "review-converge-round:v1", "state:v1 blocked" }, { "merge-ready", "fixing", "review-meta", "blocked", "reviewing" }),
    budget = budget(150, "The long review receiver is supervised by review-converge-round heartbeats; this budget only bounds stale heartbeat redrive."),
    liveness_contract = liveness({
      mode = "live-defer",
      signal = {
        family = "review-converge-round",
        producer = "review-converge-round",
        surface = "pr-comment-stream",
        version_form = "safe_version_segment",
        max_age_minutes = 120,
      },
    }),
    on_timeout = timeout("devloop_reviewing"),
    responsibility_signature = responsibility_signature({
      receiver_kind = "reviewer",
      driving_queue = "devloop_reviewing",
      state_kind = "decision",
      liveness_class = "reviewing.active",
      input_fact_family = "pr-revision-review-request",
      output_postcondition_family = "review_decision_recorded",
      decision_type = "ReviewDecision",
      phase_rank = M.stage_rank("reviewing"),
      lineage_keys = { "state.version", "pr-link.pr", "pr-head.sha", "source_ref" },
      successors = {
        {
          state = "merge-ready",
          output_variant = "approved",
          postcondition_family = "review_decision_recorded",
          decision_type = "ReviewDecision",
          monotonic = true,
        },
        {
          state = "fixing",
          output_variant = "changes_requested",
          postcondition_family = "review_decision_recorded",
          decision_type = "ReviewDecision",
          bump = true,
        },
        {
          state = "review-meta",
          output_variant = "needs_review_meta",
          postcondition_family = "review_decision_recorded",
          decision_type = "ReviewDecision",
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
    payload_builder = payloads_builders.build_devloop_reviewing_payload,
    dedup_shape = "reviewing/<proposal_id>/<state.version>/<pr>",
    required_facts = {
      fact("state", "marker-read"),
      fact("pr-link", "marker-read"),
      fact("pr-head", "fetch-before-compare"),
      fact("review-converge-round", "marker-read"),
    },
    advancing_facts = {
      advancing_fact("review-result", "merge-ready", { pr = true, liveness_scan = true }, "source_ref:pr"),
      advancing_fact("review-result", "fixing", { pr = true, liveness_scan = true }, "source_ref:pr"),
      advancing_fact("review-converge-round", "review-meta", { pr = true, liveness_scan = true }, "source_ref:pr"),
      advancing_fact("review-converge-round", "blocked", { pr = true, liveness_scan = true }, "source_ref:pr"),
    },
    payload_fields = {
      proposal_id = "marker:state.proposal",
      version = "marker:state.version",
      pr_number = "marker:pr-link.pr",
      source_ref = "source_ref:pr",
    },
    version_identity = "strip_transition_version_suffixes(state.version)",
    effects = effect(
      { "devloop_reviewing", "pr-state-label" },
      "reviewing replay is complete when current PR head is fetched, no head-bound review result exists, and the PR-local state label projection is requested",
      "build_reconcile_pr_state_label_request"
    ),
    marker_facts = "state:v1 reviewing plus PR head facts",
    kickoff = "devloop_reviewing",
    replay = "PR observe re-derives review kickoff from current PR head and issue version.",
  }
end
