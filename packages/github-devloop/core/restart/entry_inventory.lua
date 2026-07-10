return {
  {
    id = "github-devloop/thinking/entry/unmanaged_issue",
    owner = "github-devloop",
    row_id = "thinking",
    kind = "entry",
    source = {
      state = nil,
      boundary = "github-proxy.github_entity_changed",
    },
    target = "thinking",
    provenance = {
      owner = "github-devloop",
      row = "thinking",
      field = "entry_inventory.unmanaged_issue",
    },
  },
  {
    id = "github-devloop/thinking/entry/execute_request",
    owner = "github-devloop",
    row_id = "thinking",
    kind = "entry",
    source = {
      state = nil,
      boundary = "github-devloop.devloop_execute_request",
    },
    target = "thinking",
    provenance = {
      owner = "github-devloop",
      row = "thinking",
      field = "entry_inventory.execute_request",
    },
  },
}
