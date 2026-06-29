local M = {}
local wiring = require("core.devloop_wiring")

require("devloop.base").install(M)
require("devloop.config").install(M)
require("forge.github_debug_stamp").install(M)
require("devloop.strings").install(M)
require("devloop.commands").install(M)
require("devloop.entity_list_cache").install(M)
require("devloop.github_proxy_entity_view").install(M)
require("devloop.parsers").install(M)
require("devloop.logging").install(M)
require("devloop.conflict_telemetry").install(M)
require("devloop.state").install(M)
require("devloop.markers").install(M)
require("core.intake_service_class").install(M)
require("devloop.payloads").install(M)
require("devloop.decompose").install(M)
local prompts = require("devloop.prompts")
prompts.install(M, wiring.prompts(), {
  intake = true,
  intake_parser = true,
})
require("core.intake_class").install(M)
require("devloop.requests").install(M)
require("devloop.entity").install(M)
require("devloop.validators").install(M)
require("devloop.context_bundle").install(M)
require("devloop.operator_commands").install(M)
require("devloop.claims").install(M)
require("devloop.saga_conformance").install(M)

return M
