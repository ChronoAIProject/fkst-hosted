return {
  {
    id = "github-devloop-pr/reviewing/operator_reentry/rereview_blocked",
    owner = "github-devloop-pr",
    row_id = "reviewing",
    kind = "operator_reentry",
    source = {
      state = "blocked",
      boundary = nil,
    },
    target = "reviewing",
    pending_order = { participates = false },
    cause_evidence = {
      command = "rereview",
      requires_applied_certificate = true,
      resolver = "operator_commands",
    },
    provenance = {
      owner = "github-devloop-pr",
      row = "reviewing",
      field = "operator_reentry_inventory.rereview_blocked",
    },
  },
  {
    id = "github-devloop-pr/reviewing/operator_reentry/rereview_review_meta",
    owner = "github-devloop-pr",
    row_id = "reviewing",
    kind = "operator_reentry",
    source = {
      state = "review-meta",
      boundary = nil,
    },
    target = "reviewing",
    pending_order = { participates = false },
    cause_evidence = {
      command = "rereview",
      requires_applied_certificate = true,
      resolver = "operator_commands",
    },
    provenance = {
      owner = "github-devloop-pr",
      row = "reviewing",
      field = "operator_reentry_inventory.rereview_review_meta",
    },
  },
  {
    id = "github-devloop-pr/reviewing/operator_reentry/rereview_reviewing",
    owner = "github-devloop-pr",
    row_id = "reviewing",
    kind = "operator_reentry",
    source = {
      state = "reviewing",
      boundary = nil,
    },
    target = "reviewing",
    pending_order = { participates = false },
    cause_evidence = {
      command = "rereview",
      requires_applied_certificate = true,
      resolver = "operator_commands",
    },
    provenance = {
      owner = "github-devloop-pr",
      row = "reviewing",
      field = "operator_reentry_inventory.rereview_reviewing",
    },
  },
}
