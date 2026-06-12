return function(M, h)
  local fact = h.fact
  local obligation = h.obligation
  local effect = h.effect
  local budget = h.budget
  local timeout = h.timeout
  return {
    from_state = "implementing",
    terminal = false,
    to_states = { "pr-open", "impl-failed" },
    driving_queue = "github-proxy.github_entity_changed",
    output_obligation = obligation({ "state:v1 pr-open", "state:v1 impl-failed" }, { "pr-open", "impl-failed" }),
    budget = budget(45),
    on_timeout = timeout("github-proxy.github_entity_changed"),
    payload_builder = M.build_devloop_open_pr_payload,
    dedup_shape = "open-pr-kickoff/<proposal_id>/<impl_version>/<branch>",
    required_facts = {
      fact("state", "marker-read"),
      fact("implementing", "marker-read"),
      fact("implement-attempt", "marker-read"),
      fact("branch-head", "fetch-before-compare"),
    },
    payload_fields = {
      proposal_id = "marker:implementing.proposal",
      version = "marker:implementing.dedup",
      branch = "marker:implementing.branch",
      head_sha = "marker:implementing.head_sha",
      base_branch = "marker:implementing.base_branch",
      source_ref = "source_ref:issue",
    },
    version_identity = "implementing.dedup",
    effects = effect({ "devloop_open_pr" }, "implementing transition is complete only when branch progress immediately kicks off PR opening"),
    marker_facts = "state:v1 implementing plus implementing:v1 and implement-attempt:v1",
    kickoff = "devloop_open_pr",
    replay = "Observe re-raises devloop_ready only when the implement attempt is past its liveness budget; implement then re-derives PR link, remote branch, local branch, or bounded retry.",
  }
end
