local M = {}

function M.persistence_class()
  return "saga"
end

function M.pr_package_queue(queue)
  return tostring(queue)
end

function M.decompose_package_queue()
  return "github-devloop-decompose.devloop_decompose"
end

require("std.devloop_base").install(M)
require("std.devloop_config").install(M)
require("std.github_debug_stamp").install(M)
require("std.devloop_strings").install(M)
require("std.devloop_commands").install(M)
require("std.devloop_entity_list_cache").install(M)
require("std.devloop_github_proxy_entity_view").install(M)
require("std.devloop_git_mechanics").install(M)
require("std.devloop_parsers").install(M)
require("std.devloop_github_risk").install(M)
require("std.devloop_autonomy_ledger").install(M)
require("std.devloop_merge_gate").install(M)
require("std.devloop_merge_gate_wait").install(M)
require("core.review_carry_over").install(M)
require("std.devloop_merge_queue").install(M)
require("std.devloop_merge_batch").install(M)
require("std.devloop_logging").install(M)
require("std.devloop_conflict_telemetry").install(M)
require("std.devloop_state").install(M)
require("std.devloop_markers").install(M)
require("std.devloop_pr_safety").install(M)
require("std.devloop_payloads").install(M)
require("std.devloop_convergence").install(M)
require("std.devloop_decompose").install(M)
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
M.restart_marker_fields_index = "core.restart.marker_fields.index"
M.restart_replay_payload_fields_index = "core.restart.required_replay_payload_fields.index"
M.restart_transitions_index = "core.restart.transitions.index"
M.restart_liveness_signal_producers_index = "core.restart.liveness_signal_producers.index"
M.restart_source_root = "packages/github-devloop-pr/"
M.restart_consumer_sources = {
  "packages/github-devloop-pr/departments/observe_pr/main.lua",
  "packages/github-devloop-pr/departments/merge/main.lua",
}
require("std.devloop_restart").install(M)
require("std.devloop_restart_liveness_contract").install(M)
require("std.devloop_restart_responsibility_contract").install(M)
require("std.devloop_restart_actionable_epoch").install(M)
require("core.review_redrive").install(M)
require("core.pr_review_replayer").install(M)
require("std.devloop_replayer").install(M)
require("std.devloop_liveness").install(M)
require("std.devloop_sweep_bounds").install(M)
require("std.devloop_liveness_scan").install(M)
require("std.devloop_prompts").install(M)
require("std.devloop_requests").install(M)
require("core.pr_label_requests").install(M)
require("core.review_meta_requests").install(M)
require("std.devloop_entity").install(M)
require("std.devloop_validators").install(M)
require("std.devloop_queue_starvation").install(M)
require("std.devloop_context_bundle").install(M)
require("std.devloop_operator_commands").install(M)
require("std.devloop_claims").install(M)

return M
