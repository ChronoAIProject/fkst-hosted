local h = require("tests.devloop_core_helpers")
local recovery_evidence = require("devloop.recovery_evidence")

local core = h.core
local t = h.t

local function contains(errors, needle)
  for _, message in ipairs(errors or {}) do
    if tostring(message):find(needle, 1, true) ~= nil then
      return true
    end
  end
  return false
end

local function copy_inventory(inventory)
  local copied = {}
  for index, entry in ipairs(inventory or {}) do
    copied[index] = {
      state = entry.state,
      evidence = entry.evidence,
      run = entry.run,
    }
  end
  return copied
end

return {
  test_current_restart_rows_have_executable_recovery_evidence = function()
    local errors = recovery_evidence.errors(
      core,
      core.restart_transition_table(),
      core.restart_recovery_evidence_inventory
    )
    t.eq(#errors, 0, table.concat(errors, "\n"))
  end,

  test_recovery_evidence_rejects_missing_row = function()
    local inventory = copy_inventory(core.restart_recovery_evidence_inventory)
    local missing = table.remove(inventory, 1).state
    local errors = recovery_evidence.errors(core, core.restart_transition_table(), inventory)
    t.is_true(contains(errors, missing .. ": non-terminal restart row is missing recovery evidence"))
  end,

  test_recovery_evidence_rejects_unknown_row = function()
    local inventory = copy_inventory(core.restart_recovery_evidence_inventory)
    table.insert(inventory, recovery_evidence.bind("not-a-restart-state"))
    local errors = recovery_evidence.errors(core, core.restart_transition_table(), inventory)
    t.is_true(contains(errors, "not-a-restart-state: recovery evidence names an unknown non-terminal restart row"))
  end,

  test_recovery_evidence_rejects_duplicate_row = function()
    local inventory = copy_inventory(core.restart_recovery_evidence_inventory)
    table.insert(inventory, recovery_evidence.bind(inventory[1].state))
    local errors = recovery_evidence.errors(core, core.restart_transition_table(), inventory)
    t.is_true(contains(errors, inventory[1].state .. ": duplicate recovery evidence entry"))
  end,

  test_recovery_evidence_rejects_testless_row = function()
    local inventory = copy_inventory(core.restart_recovery_evidence_inventory)
    inventory[1].run = nil
    local errors = recovery_evidence.errors(core, core.restart_transition_table(), inventory)
    t.is_true(contains(errors, inventory[1].state .. ": recovery evidence entry has no executable production replay binding"))
  end,
}
