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
    effects = effect({ "github-proxy.github_entity_changed" }, "open-pr payload is complete when implementing marker and fetched branch head agree"),
    marker_facts = "state:v1 implementing plus implementing:v1",
    kickoff = "github-proxy.github_entity_changed",
    replay = "Branch poll re-derives PR open or impl-failed from branch/worktree facts.",
  }
end
