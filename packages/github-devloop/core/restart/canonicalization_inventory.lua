return {
  {
    id = "github-devloop/dependency_wait/canonicalization/legacy_ready_dependency_hold",
    owner = "github-devloop",
    row_id = "dependency_wait",
    kind = "canonicalization",
    source = {
      state = "ready",
      boundary = nil,
    },
    target = "dependency_wait",
    cause_evidence = {
      marker = "ready-split-canonicalized:v1",
      resolver = "ready_split_canonicalized_fact",
    },
    provenance = {
      owner = "github-devloop",
      row = "dependency_wait",
      field = "canonicalization_inventory.legacy_ready_dependency_hold",
    },
  },
  {
    id = "github-devloop/ready/canonicalization/legacy_ready_rederive",
    owner = "github-devloop",
    row_id = "ready",
    kind = "canonicalization",
    source = {
      state = "ready",
      boundary = nil,
    },
    target = "ready",
    cause_evidence = {
      marker = "ready-split-canonicalized:v1",
      resolver = "ready_split_canonicalized_fact",
    },
    provenance = {
      owner = "github-devloop",
      row = "ready",
      field = "canonicalization_inventory.legacy_ready_rederive",
    },
  },
  {
    id = "github-devloop/awaiting-pr/canonicalization/implementing_merged_delegated_pr",
    owner = "github-devloop",
    row_id = "awaiting-pr",
    kind = "canonicalization",
    source = {
      state = "implementing",
      boundary = nil,
    },
    target = "awaiting-pr",
    cas_policy_id = "cas.legacy_awaiting_pr_v1",
    cas_variant = "implementing_to_awaiting_pr",
    cause_evidence = {
      marker = "pr-delegation:v1",
      resolver = "pr_delegation_fact",
    },
    provenance = {
      owner = "github-devloop",
      row = "awaiting-pr",
      field = "canonicalization_inventory.implementing_merged_delegated_pr",
    },
  },
  {
    id = "github-devloop/awaiting-pr/canonicalization/legacy_pr_open_delegation",
    owner = "github-devloop",
    row_id = "awaiting-pr",
    kind = "canonicalization",
    source = {
      state = "pr-open",
      boundary = nil,
    },
    target = "awaiting-pr",
    cause_evidence = {
      marker = "pr-delegation:v1",
      resolver = "pr_delegation_fact",
    },
    provenance = {
      owner = "github-devloop",
      row = "awaiting-pr",
      field = "canonicalization_inventory.legacy_pr_open_delegation",
    },
  },
}
