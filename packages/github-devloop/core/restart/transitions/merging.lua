return function(M, h)
  local fact = h.fact
  local obligation = h.obligation
  local effect = h.effect
  local budget = h.budget
  local timeout = h.timeout
  return {
    from_state = "merging",
    terminal = false,
    to_states = { "merged", "fixing", "blocked" },
    driving_queue = "devloop_merge_ready",
    output_obligation = obligation({ "merged:v1", "state:v1 fixing", "state:v1 blocked" }, { "merged", "fixing", "blocked" }),
    budget = budget(45),
    on_timeout = timeout("devloop_merge_ready"),
    payload_builder = M.build_devloop_merge_ready_payload,
    dedup_shape = "merge-ready/<proposal_id>/<version>/<pr>/<review_dedup>/<current_head>",
    required_facts = {
      fact("state", "marker-read"),
      fact("merge-ready", "marker-read"),
      fact("merging", "marker-read"),
      fact("review-result", "marker-read"),
      fact("pr-head", "fetch-before-compare"),
      fact("ci-status", "fetch-before-compare"),
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
