local payloads_builders = require("devloop.payloads.builders")
return function(M, h)
  local fact = h.fact
  local obligation = h.obligation
  local effect = h.effect
  local budget = h.budget
  local timeout = h.timeout
  local liveness = h.liveness
  local responsibility_signature = h.responsibility_signature; local span_contract = h.span_contract
  local advancing_fact = h.advancing_fact
  return {
    from_state = "review-meta",
    liveness_class_id = "review_meta.actionable",
    watchdog = {
      mode = "live-defer",
      budget_ms = 90 * 60 * 1000,
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
    to_states = { "fixing", "blocked" },
    driving_queue = "devloop_review_meta",
    observe_surfaces = { issue = true, pr = true, liveness_scan = true },
    output_obligation = obligation({ "review-meta:v1", "state:v1 fixing", "state:v1 blocked" }, { "fixing", "blocked" }),
    budget = budget(90, "A live review-meta codex defers when fkst.codex_runs() positively reports a matching run with an unexpired run-derived deadline, or when codex run liveness is indeterminate; only positively not-running status falls back to the marker-budget timeout path."),
    liveness_contract = liveness({
      mode = "live-defer",
      real_execution = {
        primitive = "fkst.codex_runs",
        match = {
          role = "review-meta",
          proposal_id = "state.proposal_id",
          dedup_key = "state.version",
        },
        status = "running",
        on_error = "defer",
      },
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
    payload_builder = payloads_builders.build_devloop_review_meta_payload,
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
    advancing_facts = {
      advancing_fact("review-meta", "fixing", { pr = true, liveness_scan = true }, "source_ref:pr"),
      advancing_fact("review-meta", "blocked", { pr = true, liveness_scan = true }, "source_ref:pr"),
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
    span_contract = span_contract({
      department = "review_meta",
      durable_start_marker = "state:v1 review-meta",
      spawn_function = "review_meta_codex_decision",
    }),
  }
end
