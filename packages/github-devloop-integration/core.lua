local M = {}
local wiring = require("core.devloop_wiring")

function M.persistence_class()
  return "saga"
end

require("devloop.base").install(M)
require("devloop.config").install(M)
require("forge.github_debug_stamp").install(M)
require("devloop.strings").install(M)
require("devloop.commands").install(M)
require("devloop.git_mechanics").install(M)
require("devloop.parsers").install(M)
require("devloop.pr_safety").install(M)
require("devloop.merge_gate").install(M)
require("devloop.logging").install(M)
require("devloop.conflict_telemetry").install(M)
require("devloop.state").install(M)
require("devloop.markers").install(M)
require("devloop.payloads").install(M)
require("devloop.entity").install(M)
require("devloop.claims").install(M)
require("devloop.prompts").install(M, wiring.prompts())
require("devloop.github_proxy_entity_view").install(M)
require("core.branches").install(M)
require("core.sync_conflict").install(M)
require("core.rollup_health").install(M)
require("core.release_notes").install(M)
require("core.substrate_ref").install(M)

return M
