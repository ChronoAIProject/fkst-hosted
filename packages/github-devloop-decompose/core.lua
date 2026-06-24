local M = {}
local wiring = require("core.devloop_wiring")


function M.decompose_package_queue()
  return "devloop_decompose"
end

require("devloop.base").install(M)
require("devloop.config").install(M)
require("forge.github_debug_stamp").install(M)
require("devloop.strings").install(M)
require("devloop.commands").install(M)
require("devloop.entity_list_cache").install(M)
require("devloop.github_proxy_entity_view").install(M)
require("devloop.github_risk").install(M)
require("devloop.parsers").install(M)
require("devloop.logging").install(M)
require("devloop.state").install(M)
require("devloop.markers").install(M)
require("devloop.payloads").install(M)
require("devloop.convergence").install(M)
require("devloop.decompose").install(M)
local prompts = require("devloop.prompts")
prompts.install(M, wiring.prompts(), { decompose = true })
require("devloop.entity").install(M)
require("devloop.validators").install(M)
require("devloop.context_bundle").install(M)
require("devloop.claims").install(M)
require("core.saga").install(M)
require("core.decompose").install(M)

return M
