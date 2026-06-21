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
require("core.github_capabilities").install(M)
require("core.github_graphql").install(M)
require("std.devloop_commands").install(M)
require("std.devloop_entity_list_cache").install(M)
require("std.devloop_github_proxy_entity_view").install(M)
require("std.devloop_github_risk").install(M)
require("std.devloop_pr_safety").install(M)
require("std.devloop_queue").install(M)
require("std.devloop_merge_gate").install(M)
require("std.devloop_merge_gate_wait").install(M)
require("std.devloop_merge_queue").install(M)
require("std.devloop_merge_batch").install(M)
require("std.devloop_queue_starvation").install(M)
require("core.branches").install(M)
require("std.devloop_claims").install(M)
require("core.forks").install(M)
require("std.devloop_parsers").install(M)
require("std.devloop_autonomy_ledger").install(M)
require("std.devloop_logging").install(M)
require("std.devloop_conflict_telemetry").install(M)
require("core.error_facts").install(M)
require("core.failure_triage").install(M)
require("core.conflict_telemetry").install(M)
require("core.sync_conflict").install(M)
require("std.devloop_state").install(M)
require("core.state_gap").install(M)
require("std.devloop_markers").install(M)
require("core.pr_delegation").install(M)
require("core.intake_service_class").install(M)
require("core.intake_scan").install(M)
require("core.impl_failure").install(M)
require("std.devloop_payloads").install(M)
require("std.devloop_convergence").install(M)
require("std.devloop_decompose").install(M)
require("core.saga").install(M)
require("core.decompose").install(M)
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
M.restart_marker_fields_index = "core.restart.marker_fields.index"
M.restart_replay_payload_fields_index = "core.restart.required_replay_payload_fields.index"
M.restart_transitions_index = "core.restart.transitions.index"
M.restart_liveness_signal_producers_index = "core.restart.liveness_signal_producers.index"
M.restart_source_root = "packages/github-devloop/"
M.restart_consumer_sources = {
  "packages/github-devloop/departments/observe_issue/main.lua",
  "packages/github-devloop/departments/liveness_scan/main.lua",
  "packages/github-devloop/core/awaiting_pr_replayer.lua",
  "packages/github-devloop/core/ready_split.lua",
  "std/devloop_decompose.lua",
}
require("std.devloop_restart").install(M)
require("core.restart.pr_partition_contract").install(M)
require("std.devloop_restart_liveness_contract").install(M)
require("std.devloop_restart_responsibility_contract").install(M)
require("std.devloop_restart_actionable_epoch").install(M)
require("core.ready_split").install(M)
require("core.awaiting_pr_replayer").install(M)
require("std.devloop_replayer").install(M)
require("std.devloop_liveness").install(M)
require("std.devloop_liveness_scan").install(M)
require("std.devloop_prompts").install(M)
require("core.intake_class").install(M)
require("std.devloop_requests").install(M)
require("core.reconcile_requests").install(M)
require("std.devloop_entity").install(M)
require("core.implement_attempt").install(M)
require("core.ratchet_slice_ledger").install(M)
require("core.dependencies").install(M)
require("std.devloop_validators").install(M)
require("std.devloop_sweep_bounds").install(M)
require("core.observability_bounds").install(M)
require("core.rollup_health").install(M)
require("core.observability").install(M)
require("core.release_notes").install(M)
require("core.ensure_repo").install(M)
require("core.substrate_ref").install(M)
require("std.devloop_context_bundle").install(M)
require("std.devloop_operator_commands").install(M)
require("core.doctor").install(M)

return M
