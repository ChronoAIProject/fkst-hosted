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
require("core.prompts").install(M)
require("core.requests").install(M)
require("core.validators").install(M)

return M
