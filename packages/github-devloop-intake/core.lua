local saga_conformance = require("devloop.saga_conformance")
local M

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
require("core.admission").install(M)
require("devloop.entity").install(M)
require("devloop.claims").install(M)

return M
