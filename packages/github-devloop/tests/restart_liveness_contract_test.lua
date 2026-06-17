local h = require("tests.devloop_core_helpers")
local core = h.core
local t = h.t

local function copy_value(value)
  if type(value) ~= "table" then
    return value
  end
  local out = {}
  for key, nested in pairs(value) do
    out[key] = copy_value(nested)
  end
  return out
end

local function copy_rows(rows)
  local copied = {}
  for index, row in ipairs(rows or {}) do
    copied[index] = copy_value(row)
  end
  return copied
end

local function rows_by_state(rows)
  local by_state = {}
  for _, row in ipairs(rows or {}) do
    by_state[row.from_state] = row
  end
  return by_state
end

local function joined_errors(errors)
  return table.concat(errors or {}, "\n")
end

local function contains_error(errors, needle)
  return joined_errors(errors):find(needle, 1, true) ~= nil
end

local function assert_inventory_errors(inventory, state, expected)
  local listed = inventory[state]
  t.eq(type(listed), "table", state)
  local count = 0
  for err, enabled in pairs(listed) do
    t.eq(enabled, true, err)
    t.is_true(expected[err] == true, err)
    count = count + 1
  end
  local expected_count = 0
  for err, _ in pairs(expected) do
    t.is_true(listed[err] == true, err)
    expected_count = expected_count + 1
  end
  t.eq(count, expected_count, state)
end

local function synthetic_live_defer_row()
  return {
    from_state = "synthetic-live-defer-bad",
    terminal = false,
    liveness_class_id = "synthetic.bad",
    watchdog = {
      mode = "live-defer",
      budget_ms = 45 * 60 * 1000,
    },
    defer = {
      live_marker = "synthetic-live:v1",
      freshness_ms = 60 * 60 * 1000,
      clear_fact = "synthetic-clear:v1",
      observed_fact = "synthetic-observed:v1",
      clear_opens_generation = true,
    },
    budget = {
      minutes = 45,
      receiver_max_work_justification = "Synthetic fixture only.",
    },
  }
end

return {
  test_primitive_epoch_source_registry_matches_contract = function()
    local sources = core.restart_liveness_epoch_sources()
    t.eq(sources["state_entry:v1"].durable, true)
    t.eq(sources["state_entry:v1"].opens_generation, true)
    t.eq(sources["state_entry:v1"].excludes_deferred_time, false)
    t.eq(sources["state_entry:v1"].allowed_when, "no_defer_possible")
    t.eq(sources["liveness_substate_entry:v1"].durable, true)
    t.eq(sources["liveness_substate_entry:v1"].opens_generation, true)
    t.eq(sources["liveness_substate_entry:v1"].excludes_deferred_time, true)
    t.eq(sources["liveness_substate_entry:v1"].allowed_when, "hierarchical_liveness_substate")
    t.eq(sources["defer_clear_fact:v1"].durable, true)
    t.eq(sources["defer_clear_fact:v1"].opens_generation, true)
    t.eq(sources["defer_clear_fact:v1"].excludes_deferred_time, true)
    t.eq(sources["defer_clear_fact:v1"].requires_clear_fact, true)
    t.eq(sources["live_defer_epoch:v1"].durable, true)
    t.eq(sources["live_defer_epoch:v1"].opens_generation, true)
    t.eq(sources["live_defer_epoch:v1"].excludes_deferred_time, true)
    t.eq(sources["live_defer_epoch:v1"].requires_live_marker, true)
    t.eq(sources["live_defer_epoch:v1"].requires_clear_fact, true)
    t.eq(sources["live_defer_epoch:v1"].requires_observed_fact, true)
  end,

  test_row_budget_rows_declare_state_entry_actionable_epoch = function()
    local by_state = rows_by_state(core.restart_transition_table())
    for _, state in ipairs({ "pr-open", "fixing", "merge-ready", "merging", "review-meta", "impl-failed", "blocked" }) do
      local row = by_state[state]
      t.is_true(type(row.liveness_class_id) == "string" and row.liveness_class_id ~= "", state)
      t.eq(row.watchdog.mode, "row-budget-bounds-receiver")
      t.eq(row.watchdog.budget_ms, row.budget.minutes * 60 * 1000)
      t.eq(row.actionable_epoch.source, "state_entry:v1")
      t.eq(row.actionable_epoch.generation_source, "same_as_actionable_epoch")
      t.eq(row.defer, nil)
    end
  end,

  test_known_liveness_contract_violations_inventory_is_exact = function()
    local inventory = core.known_liveness_contract_violations()
    assert_inventory_errors(inventory, "ready", {
      ["ready: live-defer row must declare actionable_epoch.source"] = true,
      ["ready: live-defer row must declare defer"] = true,
    })
    assert_inventory_errors(inventory, "reviewing", {
      ["reviewing: live-defer row must declare actionable_epoch.source"] = true,
      ["reviewing: live-defer row must declare defer"] = true,
    })
    assert_inventory_errors(inventory, "implementing", {
      ["implementing: live-defer row must declare actionable_epoch.source"] = true,
      ["implementing: live-defer row must declare defer"] = true,
    })
    assert_inventory_errors(inventory, "thinking", {
      ["thinking: live-defer row must declare actionable_epoch.source"] = true,
      ["thinking: live-defer row must declare defer"] = true,
    })
    local count = 0
    for _ in pairs(inventory) do
      count = count + 1
    end
    t.eq(count, 4)
  end,

  test_inventory_ratchet_keeps_main_conformance_green = function()
    t.eq(#core.liveness_contract_errors(), 0)
    local strict = core.strict_restart_liveness_contract_errors()
    for _, state in ipairs({ "ready", "reviewing", "implementing", "thinking" }) do
      t.is_true(core.liveness_contract_inventory_is_listed_violation(state, strict), state)
    end
  end,

  test_inventory_ratchet_rejects_unlisted_and_stale_entries = function()
    local rows = copy_rows(core.restart_transition_table())
    local by_state = rows_by_state(rows)
    by_state["pr-open"].actionable_epoch = nil
    local errors = core.restart_liveness_inventory_errors(rows)
    t.is_true(contains_error(errors, "pr-open: non-terminal row must declare actionable_epoch.source"))

    by_state.ready.liveness_class_id = "ready.actionable"
    by_state.ready.watchdog = {
      mode = "live-defer",
      budget_ms = by_state.ready.budget.minutes * 60 * 1000,
    }
    by_state.ready.defer = {
      live_marker = "dependency-wait:v1",
      freshness_ms = 525600 * 60 * 1000,
      clear_fact = "dependency-wait-cleared:v1",
      observed_fact = "dependency-wait-observed:v1",
      clear_opens_generation = true,
    }
    by_state.ready.actionable_epoch = {
      source = "live_defer_epoch:v1",
      generation_source = "same_as_actionable_epoch",
    }
    errors = core.restart_liveness_inventory_errors(rows)
    t.is_true(contains_error(errors, "ready: listed known_liveness_contract_violations entry is stale and must be removed"))
  end,

  test_inventory_ratchet_rejects_extra_error_on_listed_state = function()
    local rows = copy_rows(core.restart_transition_table())
    local by_state = rows_by_state(rows)
    by_state.ready.liveness_class_id = ""
    local errors = core.restart_liveness_inventory_errors(rows)
    t.is_true(contains_error(errors, "ready: non-terminal row must declare liveness_class_id"))
  end,

  test_887_ready_model_fixture_is_strict_violation_until_p2_runtime_fix = function()
    local ready = rows_by_state(core.restart_transition_table()).ready
    local errors = core.strict_restart_liveness_contract_errors({ ready })
    t.is_true(contains_error(errors, "ready: live-defer row must declare actionable_epoch.source"))
    t.is_true(contains_error(errors, "ready: live-defer row must declare defer"))

    local now_seconds = core.iso_timestamp_epoch_seconds("2026-06-03T10:33:02Z")
    local due, age = core.liveness_timeout_due_with_facts(ready, {
      state = "ready",
      version = "ready/887",
      proposal_id = "github-devloop/issue/owner/repo/887",
      marker_created_at = "2026-06-03T09:45:00Z",
    }, {
      proposal_id = "github-devloop/issue/owner/repo/887",
      current = { comments = {} },
    }, now_seconds)
    t.eq(due, true)
    t.eq(age, 48)
  end,

  test_negative_control_live_defer_without_actionable_epoch_fails = function()
    local errors = core.strict_restart_liveness_contract_errors({ synthetic_live_defer_row() })
    t.is_true(contains_error(errors, "synthetic-live-defer-bad: live-defer row must declare actionable_epoch.source"))
  end,

  test_negative_control_live_defer_with_state_entry_epoch_fails = function()
    local row = synthetic_live_defer_row()
    row.actionable_epoch = {
      source = "state_entry:v1",
      generation_source = "same_as_actionable_epoch",
    }
    local errors = core.strict_restart_liveness_contract_errors({ row })
    t.is_true(contains_error(errors, "synthetic-live-defer-bad: live-defer row declares state_entry epoch source which cannot exclude deferred time"))
    t.is_true(contains_error(errors, "synthetic-live-defer-bad: state_entry:v1 is illegal for live-defer rows because deferred time can accrue before actionability"))
  end,
}
