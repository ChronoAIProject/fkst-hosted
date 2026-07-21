local h = require("tests.devloop_core_helpers")
local recovery_evidence = require("devloop.recovery_evidence")

local core = h.core
local t = h.t

return {
  test_current_pr_restart_rows_have_executable_recovery_evidence = function()
    local errors = recovery_evidence.errors(
      core,
      core.restart_transition_table(),
      core.restart_recovery_evidence_inventory
    )
    t.eq(#errors, 0, table.concat(errors, "\n"))
  end,
}
