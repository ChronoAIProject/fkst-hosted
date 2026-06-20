return function(M, h)
  local fact = h.fact
  local obligation = h.obligation
  local effect = h.effect
  local budget = h.budget
  local timeout = h.timeout
  local liveness = h.liveness
  local responsibility_signature = h.responsibility_signature
  local contract = require("core.restart.pr_partition_contract").awaiting_pr_contract()
  local terminal_states = contract.child_terminal_states
  return {
    from_state = "awaiting-pr",
    liveness_class_id = "child_workflow_wait",
    watchdog = {
      mode = "live-defer",
      budget_ms = 180 * 24 * 60 * 60 * 1000,
      on_stale = {
        op = "redrive_receiver",
        producer = "child-state",
      },
    },
    actionable_epoch = {
      source = "child_workflow_wait:v1",
      generation_source = "same_as_actionable_epoch",
      live_marker = "state:v1",
      producer = "child-state",
    },
    defer = {
      kind = "child_workflow_wait",
      live_marker = "state:v1",
      producer = "child-state",
      freshness_ms = 24 * 60 * 60 * 1000,
      redrive_opens_generation = true,
      delegation_marker = "pr-delegation:v1",
      terminal_states = terminal_states,
    },
    terminal = false,
    to_states = { "merged", "ready", "blocked" },
    driving_queue = contract.queue_in,
    observe_surfaces = { issue = true, pr = true, liveness_scan = true },
    output_obligation = obligation({ "state:v1 merged", "state:v1 ready", "state:v1 blocked" }, { "merged", "ready", "blocked" }),
    budget = budget(180 * 24 * 60, "The parent issue delegates PR work to a child workflow and waits on the PR child's state:v1 marker; PR review and merge time is deferred by child_workflow_wait rather than charged to the parent."),
    liveness_contract = liveness({
      mode = "live-defer",
      signal = {
        family = "state",
        resolver = "child-state",
        producer = "child-state",
        surface = "pr-comment-stream",
        version_form = "raw",
        max_age_minutes = 24 * 60,
      },
    }),
    on_timeout = timeout(contract.queue_in),
    responsibility_signature = responsibility_signature({
      receiver_kind = "pr-child-workflow",
      driving_queue = contract.queue_in,
      state_kind = "queue_wait",
      liveness_class = "child_workflow_wait",
      input_fact_family = "pr-delegation",
      output_postcondition_family = "parent_resume_from_pr_terminal",
      phase_rank = M.stage_rank("awaiting-pr"),
      lineage_keys = { "state.version", "pr-delegation.pr_proposal", "pr-delegation.pr", "source_ref" },
      successors = {
        {
          state = "merged",
          output_variant = "child_pr_merged",
          postcondition_family = "parent_resume_from_pr_terminal",
          monotonic = true,
        },
        {
          state = "ready",
          output_variant = "child_pr_closed_unmerged_replaced",
          postcondition_family = "parent_resume_from_pr_terminal",
          failure = true,
          replacement = true,
          bump = true,
        },
        {
          state = "blocked",
          output_variant = "child_pr_not_merged",
          terminal = true,
          monotonic = true,
        },
      },
    }),
    dedup_shape = "pr-terminal/<proposal_id>/<state.version>/<pr>",
    required_facts = {
      fact("state", "marker-read"),
      fact("pr-delegation", "marker-read"),
      fact("child-state", "marker-read"),
    },
    payload_fields = {
      proposal_id = "marker:state.proposal",
      version = "marker:state.version",
      pr_number = "marker:pr-delegation.pr",
      pr_proposal_id = "marker:pr-delegation.pr_proposal",
      source_ref = "source_ref:pr",
    },
    version_identity = "strip_transition_version_suffixes(state.version)",
    payload_builder = M.build_devloop_pr_terminal_payload,
    effects = effect({ contract.queue_in }, "awaiting-pr replay is inert until the delegation return handler is wired; behavior-preserving Step 1A only declares the boundary contract"),
    marker_facts = "state:v1 awaiting-pr plus pr-delegation:v1",
    kickoff = contract.queue_in,
    replay = "Declared parent wait boundary for a delegated PR child; runtime delegation and return handling are wired in the follow-up step.",
  }
end
