local M = {}
local wiring = require("core.devloop_wiring")

function M.persistence_class()
  return "saga"
end

require("std.devloop_base").install(M)
require("std.devloop_config").install(M)
require("std.github_debug_stamp").install(M)
require("std.devloop_strings").install(M)
require("std.devloop_commands").install(M)
require("std.devloop_git_mechanics").install(M)
require("std.devloop_parsers").install(M)
require("std.devloop_pr_safety").install(M)
require("std.devloop_merge_gate").install(M)
require("std.devloop_logging").install(M)
require("std.devloop_conflict_telemetry").install(M)
require("std.devloop_state").install(M)
require("std.devloop_markers").install(M)
require("std.devloop_payloads").install(M)
require("std.devloop_entity").install(M)
require("std.devloop_claims").install(M)
require("std.devloop_prompts").install(M, wiring.prompts())
require("std.devloop_github_proxy_entity_view").install(M)
require("core.branches").install(M)
require("core.sync_conflict").install(M)
require("core.rollup_health").install(M)
require("core.release_notes").install(M)
require("core.substrate_ref").install(M)

return M
