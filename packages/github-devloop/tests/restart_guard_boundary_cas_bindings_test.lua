local h = require("tests.devloop_core_helpers")
local restart_cas_catalog = require("devloop.restart_cas_catalog")
local restart_edges = require("devloop.restart_edges")

local core = h.core
local t = h.t

local expected_by_id = {
  ["github-devloop/awaiting-pr/guard_boundary/child_pr_merged"] = {
    cas_policy_id = "cas.legacy_awaiting_pr_v1",
    cas_variant = "awaiting_pr_to_merged",
  },
  ["github-devloop/awaiting-pr/guard_boundary/child_pr_closed_unmerged_replaced"] = {
    cas_policy_id = "cas.legacy_awaiting_pr_v1",
    cas_variant = "awaiting_pr_to_ready",
  },
  ["github-devloop/awaiting-pr/guard_boundary/child_pr_not_merged"] = {
    cas_policy_id = "cas.legacy_awaiting_pr_v1",
    cas_variant = "awaiting_pr_to_blocked",
  },
}

return {
  test_awaiting_pr_guard_boundary_cas_metadata_shadows_bindings_653_654_655 = function()
    local edges_by_id = {}
    for _, edge in ipairs(restart_edges.extract_guard_boundary_edges(
      core.restart_package_name,
      core.restart_transition_table()
    )) do
      edges_by_id[edge.id] = edge
    end

    local bindings_by_id = {}
    for _, binding in ipairs(restart_cas_catalog.bindings()) do
      bindings_by_id[binding.id] = binding
    end

    for id, expected in pairs(expected_by_id) do
      local edge = edges_by_id[id]
      local binding = bindings_by_id[id]
      t.is_true(edge ~= nil)
      t.is_true(binding ~= nil)
      t.eq(edge.cas_policy_id, expected.cas_policy_id)
      t.eq(edge.cas_variant, expected.cas_variant)
      t.eq(edge.cas_policy_id, binding.cas_policy_id)
      t.eq(edge.cas_variant, binding.variant)
    end
  end,
}
