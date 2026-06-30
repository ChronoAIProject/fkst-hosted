local saga_conformance = require("devloop.saga_conformance")
local M
local wiring = require("core.devloop_wiring")

-- fkst.toml conformance hook: function = "core.saga_conformance_errors" (delegates to typed devloop.saga_conformance.errors)
local function saga_conformance_errors()
  return saga_conformance.errors(M)
end

M = {
  saga_conformance_errors = saga_conformance_errors,
}

require("devloop.base").install(M)
require("forge.github_debug_stamp").install(M)
require("devloop.commands").install(M)
require("devloop.github_proxy_entity_view").install(M)
require("devloop.logging").install(M)
require("devloop.state").install(M)
require("devloop.markers").install(M)
require("core.intake_service_class").install(M)
require("devloop.payloads").install(M)
local prompts = require("devloop.prompts")
prompts.install(M, wiring.prompts(), {
  intake = true,
  intake_parser = true,
})
require("core.intake_class").install(M)
require("devloop.requests").install(M)
require("devloop.entity").install(M)
require("devloop.validators").install(M)
require("devloop.claims").install(M)

return M
