return {
  {
    id = "github-devloop-pr/reviewing/entry/first_seen_pr",
    owner = "github-devloop-pr",
    row_id = "reviewing",
    kind = "entry",
    source = {
      state = nil,
      boundary = "github-proxy.github_entity_changed",
    },
    target = "reviewing",
    provenance = {
      owner = "github-devloop-pr",
      row = "reviewing",
      field = "entry_inventory.first_seen_pr",
    },
  },
  {
    id = "github-devloop-pr/reviewing/entry/review_receiver",
    owner = "github-devloop-pr",
    row_id = "reviewing",
    kind = "entry",
    source = {
      state = nil,
      boundary = "github-devloop-pr.devloop_reviewing",
    },
    target = "reviewing",
    provenance = {
      owner = "github-devloop-pr",
      row = "reviewing",
      field = "entry_inventory.review_receiver",
    },
  },
  {
    id = "github-devloop-pr/pr-open/entry/pr_open_handoff",
    owner = "github-devloop-pr",
    row_id = "pr-open",
    kind = "entry",
    source = {
      state = nil,
      boundary = "github-proxy.github_comment_written",
    },
    target = "pr-open",
    provenance = {
      owner = "github-devloop-pr",
      row = "pr-open",
      field = "entry_inventory.pr_open_handoff",
    },
  },
}
