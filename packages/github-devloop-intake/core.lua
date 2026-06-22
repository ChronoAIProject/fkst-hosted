local M = {}
local wiring = require("core.devloop_wiring")

function M.persistence_class()
  return "saga"
end

require("std.devloop_base").install(M)
require("std.devloop_config").install(M)
require("std.github_debug_stamp").install(M)
require("std.devloop_strings").install(M)
require("core.github_capabilities").install(M)
require("std.devloop_commands").install(M)
require("std.devloop_entity_list_cache").install(M)
require("std.devloop_github_proxy_entity_view").install(M)
require("std.devloop_parsers").install(M)
require("std.devloop_logging").install(M)
require("std.devloop_conflict_telemetry").install(M)
require("std.devloop_state").install(M)
require("std.devloop_markers").install(M)
require("core.intake_service_class").install(M)
require("core.intake_scan").install(M)
require("std.devloop_payloads").install(M)
require("std.devloop_decompose").install(M)
require("std.devloop_prompts").install(M, wiring.prompts())
require("core.intake_class").install(M)
require("std.devloop_requests").install(M)
require("std.devloop_entity").install(M)
require("std.devloop_validators").install(M)
require("std.devloop_context_bundle").install(M)
require("std.devloop_operator_commands").install(M)
require("std.devloop_claims").install(M)

return M
