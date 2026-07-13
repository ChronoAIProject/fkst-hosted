return {
  {
    id = "github-devloop/implementing/operator_reentry/reimplement_impl_failed",
    owner = "github-devloop",
    row_id = "implementing",
    kind = "operator_reentry",
    source = {
      state = "impl-failed",
      boundary = nil,
    },
    target = "implementing",
    pending_order = { participates = true, predecessor_state = "impl-failed" },
    cause_evidence = {
      command = "reimplement",
      requires_applied_certificate = true,
      resolver = "operator_commands",
    },
    provenance = {
      owner = "github-devloop",
      row = "implementing",
      field = "operator_reentry_inventory.reimplement_impl_failed",
    },
  },
  {
    id = "github-devloop/implementing/operator_reentry/reimplement_blocked_open_pr",
    owner = "github-devloop",
    row_id = "implementing",
    kind = "operator_reentry",
    source = {
      state = "blocked",
      boundary = "open-pr",
    },
    target = "implementing",
    pending_order = { participates = false },
    cas_policy_id = "cas.legacy_implement_activation_handoff_v1",
    cas_variant = "blocked_to_implementing",
    cause_evidence = {
      command = "reimplement",
      requires_applied_certificate = true,
      resolver = "operator_commands",
    },
    provenance = {
      owner = "github-devloop",
      row = "implementing",
      field = "operator_reentry_inventory.reimplement_blocked_open_pr",
    },
  },
  {
    id = "github-devloop/implementing/operator_reentry/reimplement_blocked_implementing_timeout_without_pr",
    owner = "github-devloop",
    row_id = "implementing",
    kind = "operator_reentry",
    source = {
      state = "blocked",
      boundary = "implementing-timeout-without-pr",
    },
    target = "implementing",
    pending_order = { participates = false },
    cas_policy_id = "cas.legacy_implement_activation_handoff_v1",
    cas_variant = "blocked_to_implementing",
    cause_evidence = {
      command = "reimplement",
      requires_applied_certificate = true,
      resolver = "operator_commands",
    },
    provenance = {
      owner = "github-devloop",
      row = "implementing",
      field = "operator_reentry_inventory.reimplement_blocked_implementing_timeout_without_pr",
    },
  },
}
