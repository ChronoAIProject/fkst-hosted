local payloads_builders = require("devloop.payloads.builders")
return function(M, h)
  local fact = h.fact
  local obligation = h.obligation
  local effect = h.effect
  local budget = h.budget
  local timeout = h.timeout
  local liveness = h.liveness
  local actionable_epoch = h.actionable_epoch
  local responsibility_signature = h.responsibility_signature; local span_contract = h.span_contract
  local advancing_fact = h.advancing_fact
  return {
    from_state = "fixing",
    generation_entry = "always",
    liveness_class_id = "fixing.actionable",
    watchdog = {
      mode = "live-defer",
      budget_ms = 120 * 60 * 1000,
      on_stale = {
        op = "redrive_receiver",
      },
    },
    actionable_epoch = {
      source = "codex_run:v1",
      generation_source = "same_as_actionable_epoch",
    },
    defer = {
      kind = "codex_run",
      redrive_opens_generation = true,
    },
    terminal = false,
    to_states = { "reviewing", "review-meta" },
    driving_queue = "devloop_fixing",
    observe_surfaces = { issue = true, pr = true, liveness_scan = true },
    output_obligation = obligation({ "fix:v1", "state:v1 reviewing", "review-meta:v1" }, { "reviewing", "review-meta", "fixing" }),
    budget = budget(120, "A live fixing codex defers when fkst.codex_runs() positively reports a matching run with an unexpired run-derived deadline, or when codex run liveness is indeterminate; only positively not-running status falls back to the marker-budget timeout path."),
    liveness_contract = liveness({
      mode = "live-defer",
      real_execution = {
        primitive = "fkst.codex_runs",
        match = {
          role = "fix",
          proposal_id = "state.proposal_id",
          dedup_key = "state.version",
        },
        status = "running",
        on_error = "defer",
      },
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
        {
          state = "review-meta",
          output_variant = "revision_failed",
          failure = true,
          monotonic = true,
        },
      },
    }),
    payload_builder = payloads_builders.build_devloop_fixing_payload,
    dedup_shape = "forward:fixing/<proposal_id>/<version>/<pr>/<review_dedup>; replay:fixing/replay/<proposal_id>/<version>/<pr>/<review_dedup>/<gate_baseline_sha-or-nobase>/<reviewed_head_sha>",
    required_facts = {
      fact("state", "marker-read"),
      fact("pr-link", "marker-read"),
      fact("review-result", "marker-read"),
      fact("review-meta", "marker-read"),
      fact("merge-gate", "marker-read"),
      fact("pr-head", "fetch-before-compare"),
    },
    advancing_facts = {
      advancing_fact("fix-feedback", "fixing", { pr = true, liveness_scan = true }, "source_ref:pr"),
      advancing_fact("fix-feedback", "reviewing", { pr = true, liveness_scan = true }, "source_ref:pr"),
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
    span_contract = span_contract({
      department = "fix",
      durable_start_marker = "state:v1 fixing",
      spawn_predecessor = "precheck_fix_write_gate",
      spawn_function = "run_fix_attempt",
    }),
  }
end
