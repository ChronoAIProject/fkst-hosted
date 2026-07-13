return {
  {
    semantic_variant = "unmanaged_issue",
    owner = "github-devloop",
    row_id = "thinking",
    kind = "entry",
    source = {
      state = nil,
      boundary = "github-proxy.github_entity_changed",
    },
    target = "thinking",
    pending_order = { participates = true, predecessor_state = "unmanaged" },
    cas_policy_id = "cas.legacy_observe_issue_entry_v1",
    cas_variant = "unmanaged_to_thinking",
    provenance = {
      owner = "github-devloop",
      row = "thinking",
      field = "entry_inventory.unmanaged_issue",
    },
  },
  {
    semantic_variant = "execute_request",
    owner = "github-devloop",
    row_id = "thinking",
    kind = "entry",
    source = {
      state = nil,
      boundary = "github-devloop.devloop_execute_request",
    },
    target = "thinking",
    pending_order = { participates = false },
    provenance = {
      owner = "github-devloop",
      row = "thinking",
      field = "entry_inventory.execute_request",
    },
  },
}
