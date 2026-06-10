local M = {}

require("core.base").install(M)
require("core.config").install(M)
require("core.commands").install(M)
require("core.branches").install(M)
require("core.parsers").install(M)
require("core.merge_gate").install(M)
require("core.logging").install(M)
require("core.state").install(M)
require("core.markers").install(M)
require("core.payloads").install(M)
require("core.convergence").install(M)
require("core.prompts").install(M)
require("core.requests").install(M)
require("core.entity").install(M)
require("core.dependencies").install(M)
require("core.validators").install(M)
require("core.decompose").install(M)
require("core.observability").install(M)
require("core.context_bundle").install(M)

return M
