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
  "fixing->review-meta",
  "fixing->reviewing",
  "merge-ready->blocked",
  "merge-ready->merging",
  "merging->blocked",
  "merging->fixing",
  "merging->merged",
  "merging->reviewing",
  "pr-open->blocked",
  "pr-open->reviewing",
  "review-meta->blocked",
  "review-meta->fixing",
  "reviewing->fixing",
  "reviewing->merge-ready",
  "reviewing->review-meta",
}, "\n")

local function yes(predecessor)
  return { participates = true, predecessor_state = predecessor }
end

local no = { participates = false }
local pending_order_goldens = {
  ["github-devloop-pr/pr-open/autonomous/review_requested"] = yes("pr-open"),
  ["github-devloop-pr/pr-open/autonomous/not_mergeable_repair"] = no,
  ["github-devloop-pr/reviewing/autonomous/approved"] = yes("reviewing"),
  ["github-devloop-pr/reviewing/autonomous/changes_requested"] = yes("reviewing"),
  ["github-devloop-pr/reviewing/autonomous/needs_review_meta"] = yes("reviewing"),
  ["github-devloop-pr/merge-ready/autonomous/fix_budget_exhausted"] = yes("merge-ready"),
  ["github-devloop-pr/merging/autonomous/merge-completed"] = yes("merging"),
  ["github-devloop-pr/merging/autonomous/head-advanced"] = yes("merging"),
  ["github-devloop-pr/merging/autonomous/merge-needs-fix"] = yes("merging"),
  ["github-devloop-pr/merging/autonomous/fix_budget_exhausted"] = yes("merging"),
  ["github-devloop-pr/fixing/autonomous/revision_published"] = yes("fixing"),
  ["github-devloop-pr/fixing/autonomous/revision_failed"] = yes("fixing"),
  ["github-devloop-pr/fixing/autonomous/fix_budget_exhausted"] = no,
  ["github-devloop-pr/review-meta/autonomous/fix"] = yes("review-meta"),
  ["github-devloop-pr/review-meta/autonomous/block"] = yes("review-meta"),
  ["github-devloop-pr/pr-open/guard_boundary/pr_base_unmanaged"] = yes("pr-open"),
  ["github-devloop-pr/merge-ready/guard_boundary/merge_gate/approval_stale"] = no,
  ["github-devloop-pr/merge-ready/guard_boundary/merge_gate/eligible_now"] = yes("merge-ready"),
  ["github-devloop-pr/merge-ready/guard_boundary/merge_gate/code_repair_needed"] = no,
  ["github-devloop-pr/merge-ready/timeout/merge_gate/watchdog_reconcile_terminal"] = no,
  ["github-devloop-pr/reviewing/timeout/watchdog_reconcile_terminal"] = no,
  ["github-devloop-pr/reviewing/entry/first_seen_pr"] = no,
  ["github-devloop-pr/reviewing/entry/review_receiver"] = no,
  ["github-devloop-pr/reviewing/entry/review_convergence_round"] = no,
  ["github-devloop-pr/pr-open/entry/pr_open_handoff"] = no,
  ["github-devloop-pr/fixing/entry/review_reject_to_blocked"] = no,
  ["github-devloop-pr/fixing/entry/bounded_fix_to_blocked"] = no,
  ["github-devloop-pr/merge-ready/entry/handoff_to_merge_gate"] = yes("merge-ready"),
  ["github-devloop-pr/merge-ready/entry/review_reject_to_blocked"] = no,
  ["github-devloop-pr/merge-ready/entry/bounded_fix_to_blocked"] = no,
  ["github-devloop-pr/merging/entry/review_reject_to_blocked"] = no,
  ["github-devloop-pr/merging/entry/bounded_fix_to_blocked"] = no,
  ["github-devloop-pr/reviewing/entry/review_reject_to_blocked"] = no,
  ["github-devloop-pr/reviewing/operator_reentry/rereview_blocked"] = no,
  ["github-devloop-pr/reviewing/operator_reentry/rereview_review_meta"] = no,
  ["github-devloop-pr/reviewing/operator_reentry/rereview_reviewing"] = no,
  ["github-devloop-pr/reviewing/canonicalization/fixing_head_renormalization"] = yes("fixing"),
  ["github-devloop-pr/reviewing/canonicalization/pr_base_unmanaged_self_heal"] = no,
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
  t.eq(#edges, 38)
  t.eq(participating, 17)
end

return {
  test_pr_owner_pending_projection_matches_frozen_source_subset_byte_for_byte = function()
    local owner = core.restart_package_name
    local rows = core.restart_transition_table()
    local projection = owner_projection.derive(owner, rows, inventories)
    t.eq(projection_bytes(projection), expected_bytes)
    assert_pending_order_goldens(owner_projection.edges(owner, rows, inventories))
  end,
}
