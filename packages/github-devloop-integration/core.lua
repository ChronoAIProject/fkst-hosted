local saga_conformance = require("devloop.saga_conformance")
local parsers_misc = require("devloop.parsers.misc")
local parsers_pr = require("devloop.parsers.pr")
local M
local wiring = require("core.devloop_wiring")

-- fkst.toml conformance hook: function = "core.saga_conformance_errors" (delegates to typed devloop.saga_conformance.errors)
local function saga_conformance_errors()
  return saga_conformance.errors(M)
end

M = {
  saga_conformance_errors = saga_conformance_errors,
}

function M.parse_pr_view_merge(stdout)
  return parsers_pr.parse_pr_view_merge(M, stdout)
end

function M.rollup_failure_gate_sha(pr)
  return parsers_misc.rollup_failure_gate_sha(M, pr)
end

require("devloop.base").install(M)
require("forge.github_debug_stamp").install(M)
require("devloop.commands").install(M)
require("forge.merge_commands").install(M)
require("devloop.git_mechanics").install(M)
require("devloop.pr_safety").install(M)
require("forge.merge").install(M)
require("devloop.logging").install(M)
require("devloop.state").install(M)
require("devloop.entity").install(M)
local prompts = require("devloop.prompts")
prompts.install(M, wiring.prompts(), { sync_conflict = true })
local github_proxy_entity_view = require("devloop.github_proxy_entity_view")
M.cached_entity_view = function(...) return github_proxy_entity_view.cached_entity_view(M, ...) end
M.fetch_pr_view_origin = function(...) return github_proxy_entity_view.fetch_pr_view_origin(M, ...) end
M.invalidate_entity_after_write = function(...) return github_proxy_entity_view.invalidate_entity_after_write(M, ...) end
require("core.branches").install(M)
require("core.sync_conflict").install(M)
require("core.rollup_health").install(M)
require("core.release_notes").install(M)

return M
