return {
  {
    id = "github-devloop-pr/reviewing/canonicalization/fixing_head_renormalization",
    owner = "github-devloop-pr",
    row_id = "reviewing",
    kind = "canonicalization",
    source = {
      state = "fixing",
      boundary = nil,
    },
    target = "reviewing",
    pending_order = { participates = true, predecessor_state = "fixing" },
    cause_evidence = {
      marker = "fix:v1",
      resolver = "has_fix_marker",
    },
    provenance = {
      owner = "github-devloop-pr",
      row = "reviewing",
      field = "canonicalization_inventory.fixing_head_renormalization",
    },
  },
  {
    id = "github-devloop-pr/reviewing/canonicalization/pr_base_unmanaged_self_heal",
    owner = "github-devloop-pr",
    row_id = "reviewing",
    kind = "canonicalization",
    source = {
      state = "blocked",
      boundary = nil,
    },
    target = "reviewing",
    pending_order = { participates = false },
    cause_evidence = {
      marker = "state:v1",
      resolver = "current_entity_state",
    },
    provenance = {
      owner = "github-devloop-pr",
      row = "reviewing",
      field = "canonicalization_inventory.pr_base_unmanaged_self_heal",
    },
  },
}
