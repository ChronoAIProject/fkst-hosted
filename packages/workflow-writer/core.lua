-- workflow-writer package-local conformance surface.
--
-- fkst.toml conformance hook: function = "core.saga_conformance_errors". This adapter
-- drives the shared level-based workflow.engine reconcile kernel, which re-derives its
-- frontier from durable GitHub facts every tick rather than stepping an in-memory
-- restart-transition state machine. It therefore declares no restart_transition_table,
-- so the typed devloop.saga_conformance checker (the same one the github-devloop saga
-- packages use) finds no restart-transition proof obligations and returns none. The hook
-- is wired -- not stubbed: if this package ever grows a restart_transition_table, the
-- shared checker will begin enforcing its responsibility/liveness contract automatically.
local saga_conformance = require("devloop.saga_conformance")
local M

local function saga_conformance_errors()
  return saga_conformance.errors(M)
end

M = {
  saga_conformance_errors = saga_conformance_errors,
}

return M
