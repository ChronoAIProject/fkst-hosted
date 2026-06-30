local M = {}
local wiring = require("core.devloop_wiring")
local workflow_ports = require("devloop.adapters.workflow_ports")
local _hidden_state_conformance = require("devloop.hidden_state_conformance")


function M.pr_package_queue(queue)
  return tostring(queue)
end

function M.decompose_package_queue()
  return "github-devloop-decompose.devloop_decompose"
end

require("devloop.base").install(M)
require("devloop.config").install(M)
require("forge.github_debug_stamp").install(M)
require("devloop.strings").install(M)
require("devloop.commands").install(M)
require("forge.merge_commands").install(M)
require("devloop.entity_list_cache").install(M)
require("devloop.github_proxy_entity_view").install(M)
require("devloop.git_mechanics").install(M)
require("devloop.parsers").install(M)
require("devloop.autonomy_ledger").install(M)
require("forge.merge").install(M)
require("devloop.merge_gate_wait").install(M)
require("core.review_carry_over").install(M)
require("devloop.merge_queue").install(M)
require("devloop.logging").install(M)
require("devloop.conflict_telemetry").install(M)
require("devloop.state").install(M)
require("devloop.markers").install(M)
require("devloop.pr_safety").install(M)
require("devloop.payloads").install(M)
require("devloop.convergence").install(M)
require("devloop.decompose").install(M)
M.restart_package_name = "github-devloop-pr"
M.restart_lifecycle_states = {
  "pr-open",
  "reviewing",
  "fixing",
  "review-meta",
  "merge-ready",
  "merging",
  "blocked",
  "closed-unmerged",
  "merged",
}
M.restart_source_root = "packages/github-devloop-pr/"
M.restart_consumer_sources = {
  "packages/github-devloop-pr/departments/observe_pr/main.lua",
  "packages/github-devloop-pr/departments/merge/main.lua",
  "packages/github-devloop-pr/departments/merge_queue/main.lua",
}
require("devloop.restart").install(M, wiring.restart(M))
local restart_liveness_resolved = require("devloop.liveness").with_restart_policy({
  runtime_provenance = {
    proposal_id = "github-devloop/issue/provenance/repo/1",
    version = "restart-liveness-provenance",
    marker_created_at = "2026-06-03T00:00:00Z",
  },
})
restart_liveness_resolved.workflow_ports = workflow_ports.from_devloop(M)
require("workflow.restart_liveness_contract").install(M, restart_liveness_resolved)
require("devloop.restart_responsibility_contract").install(M)
require("devloop.restart_actionable_epoch").install(M)
require("core.review_redrive").install(M)
local review_replayers = require("core.pr_review_replayer").install(M)
require("devloop.replayer").install({
  core = M,
  review_replayers = review_replayers,
})
require("devloop.liveness").install(M, wiring.liveness(M))
require("devloop.liveness_scan").install(M)
local prompts = require("devloop.prompts")
prompts.install(M, wiring.prompts(), {
  fix = true,
  review_meta = true,
  review_meta_parser = true,
})
require("devloop.requests").install(M)
require("core.pr_label_requests").install(M)
require("core.review_meta_requests").install(M)
require("devloop.entity").install(M)
require("devloop.validators").install(M)
require("devloop.queue_starvation").install(M)
require("devloop.context_bundle").install(M)
require("devloop.operator_commands").install(M)
require("devloop.claims").install(M)
require("core.span_conformance").install(M)

return M
