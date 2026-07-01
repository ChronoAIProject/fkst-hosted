local M = {}
local wiring = require("core.devloop_wiring")
local parsers_misc = require("devloop.parsers.misc")
local parsers_pr = require("devloop.parsers.pr")
local workflow_ports = require("devloop.adapters.workflow_ports")
local _hidden_state_conformance = require("devloop.hidden_state_conformance")


function M.decompose_package_queue()
  return "github-devloop-decompose.devloop_decompose"
end

function M.parse_pr_view_merge(stdout)
  return parsers_pr.parse_pr_view_merge(M, stdout)
end

function M.rollup_failure_gate_sha(pr)
  return parsers_misc.rollup_failure_gate_sha(M, pr)
end

require("devloop.base").install(M)
require("forge.github_debug_stamp").install(M)
require("core.github_graphql").install(M)
require("devloop.commands").install(M)
require("forge.merge_commands").install(M)
require("devloop.github_proxy_entity_view").install(M)
require("devloop.pr_safety").install(M)
require("forge.merge").install(M)
require("devloop.git_mechanics").install(M)
require("devloop.logging").install(M)
require("devloop.state").install(M)
require("devloop.gate").install({ sources = wiring.gate_sources() })
require("core.pr_delegation").install(M)
require("core.impl_failure").install(M)
M.restart_package_name = "github-devloop"
M.restart_lifecycle_states = {
  "thinking",
  "dependency_wait",
  "ready",
  "implementing",
  "awaiting-pr",
  "impl-failed",
  "blocked",
  "merged",
}
M.restart_source_root = "packages/github-devloop/"
M.restart_consumer_sources = {
  "packages/github-devloop/departments/observe_issue/main.lua",
  "packages/github-devloop/departments/liveness_scan/main.lua",
  "packages/github-devloop/core/awaiting_pr_replayer.lua",
  "packages/github-devloop/core/ready_split.lua",
  "libraries/devloop/decompose.lua",
}
require("devloop.restart").install(M, wiring.restart(M))
require("devloop.restart.issue.pr_partition_contract").install(M)
local restart_liveness_resolved = require("devloop.liveness").with_restart_policy({
  runtime_provenance = {
    proposal_id = "github-devloop/issue/provenance/repo/1",
    version = "restart-liveness-provenance",
    marker_created_at = "2026-06-03T00:00:00Z",
  },
})
restart_liveness_resolved.workflow_ports = workflow_ports.from_devloop(M)
require("workflow.restart_liveness_contract").install(M, restart_liveness_resolved)
local restart_responsibility_contract = require("devloop.restart_responsibility_contract")
M.restart_responsibility_inventory_errors = function(...) return restart_responsibility_contract.restart_responsibility_inventory_errors(M, ...) end
M.strict_restart_responsibility_contract_errors = function(...) return restart_responsibility_contract.strict_restart_responsibility_contract_errors(M, ...) end
local restart_actionable_epoch = require("devloop.restart_actionable_epoch")
M.actionable_epoch_resolve = function(...) return restart_actionable_epoch.actionable_epoch_resolve(M, ...) end
local ready_split_replayers = require("core.ready_split").install(M)
local awaiting_pr_replayers = require("core.awaiting_pr_replayer").install(M)
M.replayer_registry = {
  dependency_wait = ready_split_replayers.dependency_wait,
  ready = ready_split_replayers.ready,
  ["awaiting-pr"] = awaiting_pr_replayers["awaiting-pr"],
}
require("core.liveness_bounds").install(M)
require("devloop.liveness").install(M, wiring.liveness(M))
local prompts = require("devloop.prompts")
prompts.install(M, wiring.prompts(), { implement = true })
require("core.reconcile_requests").install(M)
require("devloop.entity").install(M)
require("core.implement_attempt").install(M)
require("core.ratchet_slice_ledger").install(M)
require("core.dependencies").install(M)
require("core.span_conformance").install(M)

return M
