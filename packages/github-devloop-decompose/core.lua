local M = {}

function M.persistence_class()
  return "saga"
end

function M.decompose_package_queue()
  return "devloop_decompose"
end

require("std.devloop_base").install(M)
require("std.devloop_config").install(M)
require("std.github_debug_stamp").install(M)
require("std.devloop_strings").install(M)
require("std.devloop_commands").install(M)
require("std.devloop_entity_list_cache").install(M)
require("std.devloop_github_proxy_entity_view").install(M)
require("std.devloop_github_risk").install(M)
require("std.devloop_parsers").install(M)
require("std.devloop_logging").install(M)
require("std.devloop_state").install(M)
require("std.devloop_markers").install(M)
require("std.devloop_payloads").install(M)
require("std.devloop_convergence").install(M)
require("std.devloop_decompose").install(M)
require("std.devloop_prompts").install(M)
require("std.devloop_entity").install(M)
require("std.devloop_validators").install(M)
require("std.devloop_context_bundle").install(M)
require("std.devloop_claims").install(M)
require("core.saga").install(M)
require("core.decompose").install(M)

return M
