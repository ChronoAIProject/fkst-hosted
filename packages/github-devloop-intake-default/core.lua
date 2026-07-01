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
local github_proxy_entity_view = require("devloop.github_proxy_entity_view")
M.cached_entity_view = function(...) return github_proxy_entity_view.cached_entity_view(M, ...) end
M.fetch_pr_view_origin = function(...) return github_proxy_entity_view.fetch_pr_view_origin(M, ...) end
M.invalidate_entity_after_write = function(...) return github_proxy_entity_view.invalidate_entity_after_write(M, ...) end
require("devloop.logging").install(M)
require("devloop.state").install(M)
require("core.intake_service_class").install(M)
local prompts = require("devloop.prompts")
prompts.install(M, wiring.prompts(), {
  intake = true,
  intake_parser = true,
})
require("core.intake_class").install(M)
require("devloop.entity").install(M)

return M
