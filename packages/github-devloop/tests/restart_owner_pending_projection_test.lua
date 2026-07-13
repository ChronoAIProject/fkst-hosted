local h = require("tests.devloop_core_helpers")
local owner_projection = require("devloop.restart_owner_pending_projection")
local inventories = {
  canonicalization = require("core.restart.canonicalization_inventory"),
  entry = require("core.restart.entry_inventory"),
  operator_reentry = require("core.restart.operator_reentry_inventory"),
}

local core = h.core
local t = h.t

local function projection_bytes(projection)
  local lines = {}
  for predecessor, targets in pairs(projection) do
    for target in pairs(targets) do
      table.insert(lines, predecessor .. "->" .. target)
    end
  end
  table.sort(lines)
  return table.concat(lines, "\n")
end

local expected_bytes = table.concat({
  "awaiting-pr->blocked",
  "awaiting-pr->merged",
  "awaiting-pr->ready",
  "dependency_wait->blocked",
  "dependency_wait->dependency_wait",
  "dependency_wait->ready",
  "impl-failed->implementing",
  "implementing->awaiting-pr",
  "implementing->impl-failed",
  "ready->blocked",
  "ready->dependency_wait",
  "ready->implementing",
  "thinking->blocked",
  "thinking->dependency_wait",
  "thinking->ready",
  "unmanaged->thinking",
}, "\n")

local function yes(predecessor)
  return { participates = true, predecessor_state = predecessor }
end

local no = { participates = false }
local pending_order_goldens = {
  ["github-devloop/thinking/autonomous/consensus-reached"] = yes("thinking"),
  ["github-devloop/thinking/autonomous/consensus-reached-dependency-held"] = yes("thinking"),
  ["github-devloop/thinking/autonomous/premise-refuted"] = no,
  ["github-devloop/thinking/autonomous/consensus-stalled"] = yes("thinking"),
  ["github-devloop/implementing/autonomous/revision_published"] = yes("implementing"),
  ["github-devloop/implementing/autonomous/revision_failed"] = yes("implementing"),
  ["github-devloop/dependency_wait/guard_boundary/blockers_still_open"] = yes("dependency_wait"),
  ["github-devloop/dependency_wait/guard_boundary/blockers_released"] = yes("dependency_wait"),
  ["github-devloop/dependency_wait/guard_boundary/dependency_resolver_stale"] = yes("dependency_wait"),
  ["github-devloop/ready/guard_boundary/blocker_reappeared"] = yes("ready"),
  ["github-devloop/awaiting-pr/guard_boundary/child_pr_merged"] = yes("awaiting-pr"),
  ["github-devloop/awaiting-pr/guard_boundary/child_pr_closed_unmerged_replaced"] = yes("awaiting-pr"),
  ["github-devloop/awaiting-pr/guard_boundary/child_pr_not_merged"] = yes("awaiting-pr"),
  ["github-devloop/ready/timeout/actionable_kickoff_timeout"] = yes("ready"),
  ["github-devloop/thinking/entry/unmanaged_issue"] = yes("unmanaged"),
  ["github-devloop/thinking/entry/execute_request"] = no,
  ["github-devloop/impl-failed/entry/retry-implementation"] = yes("impl-failed"),
  ["github-devloop/ready/entry/implementation_kicked_off"] = yes("ready"),
  ["github-devloop/thinking/entry/issue_reconcile_true_stall"] = no,
  ["github-devloop/implementing/operator_reentry/reimplement_impl_failed"] = yes("impl-failed"),
  ["github-devloop/implementing/operator_reentry/reimplement_blocked_open_pr"] = no,
  ["github-devloop/implementing/operator_reentry/reimplement_blocked_implementing_timeout_without_pr"] = no,
  ["github-devloop/dependency_wait/canonicalization/legacy_ready_dependency_hold"] = yes("ready"),
  ["github-devloop/ready/canonicalization/legacy_ready_rederive"] = no,
  ["github-devloop/awaiting-pr/canonicalization/implementing_merged_delegated_pr"] = yes("implementing"),
  ["github-devloop/awaiting-pr/canonicalization/legacy_pr_open_delegation"] = no,
}

local function assert_pending_order_goldens(edges)
  local seen = {}
  local participating = 0
  for _, edge in ipairs(edges) do
    local expected = pending_order_goldens[edge.id]
    t.is_true(expected ~= nil)
    t.eq(edge.pending_order.participates, expected.participates)
    t.eq(edge.pending_order.predecessor_state, expected.predecessor_state)
    seen[edge.id] = true
    if edge.pending_order.participates then participating = participating + 1 end
  end
  for id in pairs(pending_order_goldens) do t.eq(seen[id], true) end
  t.eq(#edges, 26)
  t.eq(participating, 19)
end

return {
  test_issue_owner_pending_projection_matches_frozen_source_subset_byte_for_byte = function()
    local owner = core.restart_package_name
    local rows = core.restart_transition_table()
    local projection = owner_projection.derive(owner, rows, inventories)
    t.eq(projection_bytes(projection), expected_bytes)
    assert_pending_order_goldens(owner_projection.edges(owner, rows, inventories))
  end,
}
