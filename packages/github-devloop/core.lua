local M = {}

function M.persistence_class()
  return "saga"
end

-- Recurrence/API waiver: `parse_name_only_paths` stays package-root API because
-- it is shared by package-root modules that receive only the installed `M`
-- surface, and `scripts/check_repo.py` forbids reintroducing local copies.
function M.parse_name_only_paths(stdout)
  local paths = {}
  local seen = {}
  for line in tostring(stdout or ""):gmatch("[^\r\n]+") do
    local path = line:gsub("^%s+", ""):gsub("%s+$", "")
    if path ~= "" and not seen[path] then
      table.insert(paths, path)
      seen[path] = true
    end
  end
  table.sort(paths)
  return paths
end

require("core.base").install(M)
require("core.config").install(M)
require("std.github_debug_stamp").install(M)
require("core.strings").install(M)
require("core.github_capabilities").install(M)
require("core.github_graphql").install(M)
require("core.commands").install(M)
require("core.entity_list_cache").install(M)
require("core.github_proxy_entity_view").install(M)
require("core.branches").install(M)
require("core.forks").install(M)
require("core.parsers").install(M)
require("core.merge_gate").install(M)
require("core.merge_gate_wait").install(M)
require("core.autonomy_ledger").install(M)
require("core.review_carry_over").install(M)
require("core.queue").install(M)
require("core.merge_batch").install(M)
require("core.logging").install(M)
require("core.error_facts").install(M)
require("core.failure_triage").install(M)
require("core.conflict_telemetry").install(M)
require("core.sync_conflict").install(M)
require("core.state").install(M)
require("core.state_gap").install(M)
require("core.markers").install(M)
require("core.pr_safety").install(M)
require("core.pr_delegation").install(M)
require("core.intake_service_class").install(M)
require("core.intake_scan").install(M)
require("core.impl_failure").install(M)
require("core.payloads").install(M)
require("core.convergence").install(M)
require("core.saga").install(M)
require("core.decompose").install(M)
require("core.restart").install(M)
require("core.restart.pr_partition_contract").install(M)
require("core.restart.liveness_contract").install(M)
require("core.restart.responsibility_contract").install(M)
require("core.restart.actionable_epoch").install(M)
require("core.review_redrive").install(M)
require("core.pr_review_replayer").install(M)
require("core.ready_split").install(M)
require("core.awaiting_pr_replayer").install(M)
require("core.replayer").install(M)
require("core.liveness").install(M)
require("core.prompts").install(M)
require("core.intake_class").install(M)
require("core.requests").install(M)
require("core.reconcile_requests").install(M)
require("core.pr_label_requests").install(M)
require("core.review_meta_requests").install(M)
require("core.entity").install(M)
require("core.implement_attempt").install(M)
require("core.ratchet_slice_ledger").install(M)
require("core.dependencies").install(M)
require("core.validators").install(M)
require("core.sweep_bounds").install(M)
require("core.observability_bounds").install(M)
require("core.rollup_health").install(M)
require("core.queue_starvation").install(M)
require("core.observability").install(M)
require("core.release_notes").install(M)
require("core.ensure_repo").install(M)
require("core.substrate_ref").install(M)
require("core.context_bundle").install(M)
require("core.operator_commands").install(M)
require("core.claims").install(M)
require("core.doctor").install(M)

return M
