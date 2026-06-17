local h = require("tests.devloop_core_helpers")
local core = h.core
local t = h.t

local function has_value(values, expected)
  for _, value in ipairs(values or {}) do
    if value == expected then
      return true
    end
  end
  return false
end

local function table_by_state()
  local by_state = {}
  for _, row in ipairs(core.restart_transition_table()) do
    by_state[row.from_state] = row
  end
  return by_state
end

return {
  test_restart_rows_declare_authoritative_observe_surfaces = function()
    local by_state = table_by_state()
    local expected = {
      thinking = { issue = true, liveness_scan = true },
      ready = { issue = true, liveness_scan = true },
      implementing = { issue = true, liveness_scan = true },
      ["impl-failed"] = { issue = true, liveness_scan = true },
      ["pr-open"] = { issue = true, pr = true, liveness_scan = true },
      reviewing = { issue = true, pr = true, liveness_scan = true },
      ["merge-ready"] = { issue = true, pr = true, liveness_scan = true },
      merging = { issue = true, pr = true, liveness_scan = true },
      fixing = { issue = true, pr = true, liveness_scan = true },
      ["review-meta"] = { issue = true, pr = true, liveness_scan = true },
      blocked = { issue = true, pr = true, liveness_scan = true },
    }
    for state, surfaces in pairs(expected) do
      local row = by_state[state]
      t.is_true(row ~= nil, state)
      for surface, enabled in pairs(surfaces) do
        t.eq(core.restart_row_observable_on(row, surface), enabled)
      end
    end
    t.eq(core.restart_row_observable_on(by_state.merged, "issue"), false)
    t.eq(#core.liveness_contract_errors(), 0)
  end,

  test_pr_not_mergeable_recovery_is_owned_by_reviewing = function()
    local by_state = table_by_state()
    t.eq(by_state["pr-open"].pr_recovery, nil)
    t.eq(has_value(by_state["pr-open"].to_states, "fixing"), false)
    local recovery = by_state.reviewing.pr_recovery.not_mergeable
    t.eq(recovery.to_state, "fixing")
    t.eq(recovery.queue, "devloop_fixing")
    t.is_true(has_value(by_state.reviewing.to_states, "fixing"))
    t.eq(by_state.fixing.pr_recovery, nil)
    t.eq(by_state["merge-ready"].pr_recovery, nil)
  end,

  test_timeout_surfaces_are_declared_separately_from_replay_surfaces = function()
    local by_state = table_by_state()
    t.eq(by_state.thinking.timeout_surfaces.issue, true)
    t.eq(by_state.thinking.timeout_surfaces.liveness_scan, true)
    t.eq(by_state.reviewing.timeout_surfaces.issue, true)
    t.eq(by_state.reviewing.timeout_surfaces.pr, true)
    t.eq(by_state["pr-open"].timeout_surfaces.issue, nil)
    t.eq(by_state["pr-open"].timeout_surfaces.issue_liveness_scan, true)
    t.eq(by_state.ready.timeout_surfaces, nil)
    t.eq(by_state["merge-ready"].timeout_surfaces, nil)
    t.eq(core.restart_observe_timeout_due(by_state.ready, "issue", {
      state = "ready",
      version = "ready/old",
      marker_created_at = "2026-06-03T00:00:00Z",
    }, {}, core.iso_timestamp_epoch_seconds("2026-06-04T01:00:00Z")), false)
  end,
}
