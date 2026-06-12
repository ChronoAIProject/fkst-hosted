return function(M, h)
  local fact = h.fact
  local obligation = h.obligation
  local effect = h.effect
  local budget = h.budget
  local timeout = h.timeout
  return {
    from_state = "pr-open",
    terminal = false,
    to_states = { "reviewing" },
    driving_queue = "devloop_reviewing",
    output_obligation = obligation({ "state:v1 reviewing", "devloop_reviewing" }, { "reviewing" }),
    budget = budget(30),
    on_timeout = timeout("devloop_reviewing"),
    payload_builder = M.build_devloop_reviewing_payload,
    dedup_shape = "reviewing/<proposal_id>/<impl_version>/<pr>",
    required_facts = {
      fact("state", "marker-read"),
      fact("pr-link", "marker-read"),
      fact("pr-head", "fetch-before-compare"),
    },
    payload_fields = {
      proposal_id = "marker:pr-link.proposal",
      pr_number = "marker:pr-link.pr",
      version = "marker:pr-link.impl_version",
      source_ref = "source_ref:pr",
    },
    version_identity = "pr-link.impl_version",
    effects = effect({ "devloop_reviewing" }, "reviewing payload is complete when linked open PR head/base still match the pr-link marker"),
    marker_facts = "state:v1 pr-open plus pr-link:v1",
    kickoff = "devloop_reviewing",
    replay = "Observe re-fetches the linked PR and raises review for the linked PR head.",
  }
end
