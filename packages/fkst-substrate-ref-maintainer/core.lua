local M = {}

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
require("devloop.entity").install(M)
require("devloop.github_proxy_entity_view").install(M)
require("core.substrate_ref").install(M)

return M
