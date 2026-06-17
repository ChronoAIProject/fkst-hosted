local h = require("tests.devloop_core_helpers")
local core = h.core
local t = h.t

local function copy_rows(rows)
  local copied = {}
  local function copy_value(value)
    if type(value) ~= "table" then
      return value
    end
    local nested = {}
    for key, nested_value in pairs(value) do
      nested[key] = copy_value(nested_value)
    end
    return nested
  end
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

return {
  test_liveness_contract_binds_live_defer_surface_and_version_form = function()
    local by_state = rows_by_state(core.restart_transition_table())
    t.eq(by_state.thinking.liveness_contract.signal.surface, "issue-comment-stream")
    t.eq(by_state.thinking.liveness_contract.signal.version_form, "raw")
    t.eq(by_state.dependency_wait.liveness_contract.signal.surface, "issue-comment-stream")
    t.eq(by_state.dependency_wait.liveness_contract.signal.version_form, "raw")
    t.eq(by_state.implementing.liveness_contract.signal.surface, "issue-comment-stream")
    t.eq(by_state.implementing.liveness_contract.signal.version_form, "raw")
    t.eq(by_state.reviewing.liveness_contract.signal.surface, "pr-comment-stream")
    t.eq(by_state.reviewing.liveness_contract.signal.version_form, "safe_version_segment")
  end,

  test_liveness_contract_binds_row_budget_progress_signal_metadata = function()
    local by_state = rows_by_state(core.restart_transition_table())
    for _, state in ipairs({ "merge-ready", "merging" }) do
      local signal = by_state[state].liveness_contract.progress_signal
      t.eq(signal.family, "merge-gate-wait")
      t.eq(signal.resolver, "merge-gate-wait")
      t.eq(signal.producer, "merge-gate-wait")
      t.eq(signal.surface, "pr-comment-stream")
      t.eq(signal.version_form, "raw")
      t.eq(signal.max_age_minutes, 360)
    end
  end,

  test_liveness_contract_rejects_live_defer_surface_or_version_form_drift = function()
    local rows = copy_rows(core.restart_transition_table())
    local row = rows_by_state(rows).reviewing
    row.liveness_contract.signal.surface = "issue-comment-stream"
    row.liveness_contract.signal.version_form = "raw"
    local errors = core.liveness_contract_errors(rows)
    local joined = table.concat(errors, "\n")
    t.is_true(joined:find("producer binding surface mismatch", 1, true) ~= nil)
    t.is_true(joined:find("producer binding version_form mismatch", 1, true) ~= nil)
  end,

  test_liveness_contract_rejects_live_defer_missing_surface_or_version_form = function()
    local rows = copy_rows(core.restart_transition_table())
    local row = rows_by_state(rows).reviewing
    row.liveness_contract.signal.surface = nil
    row.liveness_contract.signal.version_form = nil
    local errors = core.liveness_contract_errors(rows)
    local joined = table.concat(errors, "\n")
    t.is_true(joined:find("must declare surface", 1, true) ~= nil)
    t.is_true(joined:find("must declare version_form", 1, true) ~= nil)
  end,

  test_liveness_contract_rejects_row_budget_progress_signal_drift = function()
    local rows = copy_rows(core.restart_transition_table())
    local row = rows_by_state(rows)["merge-ready"]
    row.liveness_contract.progress_signal.surface = "issue-comment-stream"
    row.liveness_contract.progress_signal.max_age_minutes = nil
    local errors = core.liveness_contract_errors(rows)
    local joined = table.concat(errors, "\n")
    t.is_true(joined:find("row-budget progress_signal must declare finite max_age_minutes", 1, true) ~= nil)
    t.is_true(joined:find("producer binding surface mismatch", 1, true) ~= nil)
  end,
}
