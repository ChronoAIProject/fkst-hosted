local owner = "github-devloop"
local records = {}
local effect_kinds = {}
for _, effect_kind in ipairs(require("devloop.restart_sinks").schema().effect_kinds) do
  effect_kinds[effect_kind] = effect_kind
end

local function add(id, department, site, effect_kind, authority_class, family)
  table.insert(records, {
    id = id,
    owner = owner,
    callsite = { department = department, site = site },
    effect_kind = effect_kind,
    authority_class = authority_class,
    dedup_marker_family = family,
  })
end

local function queue(department, name, authority_class, family)
  local qualified = name:find(".", 1, true) and name or owner .. "." .. name
  add("queue:" .. qualified, department, "M.spec.produces:" .. name, "queue", authority_class, family)
end

queue("comment_handoff", "devloop_ready", "lifecycle-authoritative", "ready:v1/proposal+version")
queue("comment_handoff", "devloop_reconcile", "lifecycle-authoritative", "reconcile:v1/proposal+round")
queue("execute_start", "consensus.proposal", "lifecycle-authoritative", "consensus-proposal:v1/proposal+dedup")
queue("liveness_scan", "devloop_observe_issue", "grantless-telemetry", "observe-issue:v1/source-ref+dedup")
queue("liveness_scan", "consensus.proposal", "lifecycle-authoritative", "consensus-proposal:v1/proposal+dedup")
queue("liveness_scan", "devloop_ready", "lifecycle-authoritative", "ready:v1/proposal+version")
queue("liveness_scan", "github-devloop-decompose.devloop_decompose", "grantless-published-intent", "decompose.v1/proposal+attempt")
queue("liveness_scan", "devloop_reconcile", "lifecycle-authoritative", "reconcile:v1/proposal+round")
queue("liveness_scan", "devloop_timeout_reconcile", "lifecycle-authoritative", "timeout-reconcile:v1/proposal+state+round")
queue("loop", "consensus.proposal", "lifecycle-authoritative", "consensus-proposal:v1/proposal+dedup")
queue("observe_issue", "consensus.proposal", "lifecycle-authoritative", "consensus-proposal:v1/proposal+dedup")
queue("observe_issue", "devloop_ready", "lifecycle-authoritative", "ready:v1/proposal+version")
queue("observe_issue", "github-devloop-decompose.devloop_decompose", "grantless-published-intent", "decompose.v1/proposal+attempt")
queue("observe_issue", "devloop_reconcile", "lifecycle-authoritative", "reconcile:v1/proposal+round")
queue("observe_issue", "devloop_timeout_reconcile", "lifecycle-authoritative", "timeout-reconcile:v1/proposal+state+round")
queue("test_board_digest_probe", "board_digest_probe", "grantless-non-lifecycle", "test-probe/board-digest-request")
queue("test_board_digest_probe", "board_digest_result", "grantless-non-lifecycle", "test-probe/board-digest-result")
queue("test_cache_seed", "cache_seed", "grantless-non-lifecycle", "test-probe/cache-seed-request")
queue("test_cache_seed", "cache_seeded", "grantless-non-lifecycle", "test-probe/cache-seed-result")
queue("test_context_bundle_probe", "context_bundle_probe", "grantless-non-lifecycle", "test-probe/context-bundle-request")
queue("test_context_bundle_probe", "context_bundle_probe_result", "grantless-non-lifecycle", "test-probe/context-bundle-result")

add("adapter:structured-log", "all", "devloop_logging.log_line", "adapter", "grantless-telemetry", "structured-log:v1/dept+proposal+tag")
add("comment:issue:consensus-result", "consensus_result", "raise_result_effects.result_comment", "comment", "lifecycle-authoritative", "state:v1+result:v1;dedup=proposal/comment/logical-result")
add("label:issue:consensus-result", "consensus_result", "raise_result_effects.result_label", "label", "lifecycle-authoritative", "state-label:ready|declined;dedup=proposal/label/logical-result")
add("comment:issue:dependency-hold", "consensus_result", "raise_result_effects.dependency_hold_comment", "comment", "lifecycle-authoritative", "dependency-wait|dependency-cycle|dependency-unresolvable:v1;dedup=dependency/comment")
add("label:issue:dependency-hold", "consensus_result", "raise_result_effects.dependency_hold_label", "label", "lifecycle-authoritative", "label:fkst-dev:blocked-on-dependency;dedup=dependency/label/hold")
add("comment:issue:dependency-release", "consensus_result", "raise_result_effects.dependency_release_comment", "comment", "lifecycle-authoritative", "dependency-release:v1;dedup=dependency/comment/release")
add("comment:issue:thinking-state", "execute_start", "raise_execution_start.thinking_comment", "comment", "lifecycle-authoritative", "state:v1/thinking;dedup=execution-start/comment")
add("label:issue:thinking-state", "execute_start", "raise_execution_start.thinking_label", "label", "lifecycle-authoritative", "state-label:thinking;dedup=execution-start/label")
add("comment:issue:implementation-failed", "implement", "raise_impl_failed.comment", "comment", "lifecycle-authoritative", "state:v1/impl-failed+impl-failure:v1;dedup=implement/comment/failure")
add("label:issue:implementation-failed", "implement", "raise_impl_failed.label", "label", "lifecycle-authoritative", "state-label:impl-failed;dedup=implement/label/impl-failed")
add("comment:issue:implementation-start", "implement", "raise_implementing_state.comment", "comment", "lifecycle-authoritative", "state:v1/implementing+implement-attempt:v1;dedup=implement/comment/implementing-state")
add("label:issue:implementation-start", "implement", "raise_implementing_state.label", "label", "lifecycle-authoritative", "state-label:implementing;dedup=implement/label/implementing")
add("comment:issue:implementation-progress", "implement", "raise_implementing.comment", "comment", "lifecycle-authoritative", "implementing:v1+implement-attempt:v1;dedup=implement/comment/implementing")
add("comment:issue:implementation-attempt", "implement", "raise_implement_attempt.comment", "comment", "lifecycle-authoritative", "implement-attempt:v1;dedup=implement/comment/attempt")
add("comment:issue:implementation-version-mismatch", "implement", "raise_implement_version_mismatch.comment", "comment", "lifecycle-authoritative", "implement-version-mismatch:v1;dedup=implement/comment/version-mismatch")
add("comment:issue:dependency-canonicalization", "implement", "process_ready_event.dependency_canonicalization_comment", "comment", "lifecycle-authoritative", "state:v1/dependency_wait+ready-split-canonicalized:v1")
add("label:issue:dependency-canonicalization", "implement", "process_ready_event.dependency_hold_label", "label", "lifecycle-authoritative", "label:fkst-dev:blocked-on-dependency;dedup=dependency/label/hold")
add("comment:pr:pr-child-open", "implement", "pr_child_handoff.child_start_comment", "comment", "lifecycle-authoritative", "state:v1/pr-open+pr-origin:v1+pr-link:v1;dedup=pr-delegation/pr-open")
add("comment:issue:pr-delegation", "implement", "pr_child_handoff.issue_delegation_comment", "comment", "lifecycle-authoritative", "pr-delegation:v1;dedup=pr-delegation/issue")
add("comment:issue:awaiting-pr-state", "implement", "pr_child_handoff.parent_awaiting_comment", "comment", "lifecycle-authoritative", "state:v1/awaiting-pr+pr-delegation:v1;dedup=awaiting-pr")
add("label:issue:awaiting-pr-state", "implement", "pr_child_handoff.parent_awaiting_label", "label", "lifecycle-authoritative", "state-label:awaiting-pr;dedup=awaiting-pr/label")
add("codex.dispatch:implement", "implement", "run_attempt.codex_dispatch", "codex", "lifecycle-authoritative", "codex-run:implement/proposal+dedup+worker")
add("git.push:implementation-branch", "implement", "publish_implementation_branch.push", effect_kinds.git, "lifecycle-authoritative", "git-push/implementation-branch/proposal+version")
add("adapter:github.pr-create", "implement", "pr_child_handoff.create_pr", "adapter", "grantless-non-lifecycle", "pr-create/head+base+issue")
add("comment:issue:duplicate-slice", "implement", "slice_gate.duplicate_comment", "comment", "grantless-non-lifecycle", "duplicate-slice:v1;dedup=implement/duplicate-slice")
add("label:issue:duplicate-slice", "implement", "slice_gate.duplicate_label", "label", "grantless-non-lifecycle", "label:fkst:duplicate-slice;dedup=implement/duplicate-slice/label")
add("adapter:github.issue-close-duplicate-slice", "implement", "slice_gate.close_duplicate", "adapter", "grantless-non-lifecycle", "issue-close/duplicate-slice")
add("comment:issue:duplicate-fork", "implement", "fork_gate.duplicate_comment", "comment", "grantless-non-lifecycle", "duplicate-fork:v1;dedup=implement/duplicate-fork")
add("label:issue:duplicate-fork", "implement", "fork_gate.duplicate_label", "label", "grantless-non-lifecycle", "label:fkst:duplicate-fork;dedup=implement/duplicate-fork/label")
add("adapter:github.issue-close-duplicate-fork", "implement", "fork_gate.close_duplicate", "adapter", "grantless-non-lifecycle", "issue-close/duplicate-fork")
add("comment:issue:converge-round", "loop", "act.converge_round_comment", "comment", "lifecycle-authoritative", "converge-round:v1;dedup=converge-round/comment")
add("comment:issue:operator-refusal", "observe_issue", "operator_commands.refusal_comments", "comment", "grantless-non-lifecycle", "operator-command:v1/refused;dedup=operator-command/refusal")
add("comment:issue:operator-rereview", "observe_issue", "maybe_apply_issue_rereview_command.applied_comment", "comment", "lifecycle-authoritative", "state:v1/thinking+operator-command:v1/rereview")
add("comment:issue:operator-reimplement", "observe_issue", "maybe_apply_issue_reimplement_command.applied_comment", "comment", "lifecycle-authoritative", "operator-command:v1/reimplement;dedup=operator-command/reimplement")
add("comment:issue:dependency-waiver", "observe_issue", "maybe_apply_issue_dependency_waiver_command.applied_comment", "comment", "lifecycle-authoritative", "dependency-waiver:v1+operator-command:v1;dedup=dependency-waiver")
add("label:issue:dependency-stale-clear", "observe_issue", "raise_stale_dependency_label_clear.label", "label", "lifecycle-authoritative", "label-remove:fkst-dev:blocked-on-dependency;dedup=dependency/label/clear-stale")
add("comment:issue:thinking-state", "observe_issue", "process_issue_event.thinking_comment", "comment", "lifecycle-authoritative", "state:v1/thinking;dedup=proposal/comment/thinking")
add("label:issue:thinking-state", "observe_issue", "process_issue_event.thinking_label", "label", "lifecycle-authoritative", "state-label:thinking;dedup=proposal/label/thinking")
add("comment:issue:row-replay", "observe_issue", "process_issue_event.restart_replay_comments", "comment", "lifecycle-authoritative", "restart-row-replay/issue-comment-marker-families")
add("label:issue:row-replay", "observe_issue", "process_issue_event.restart_replay_labels", "label", "lifecycle-authoritative", "restart-row-replay/state-label-families")
add("comment:pr:row-replay", "observe_issue", "process_issue_event.restart_replay_pr_comments", "comment", "lifecycle-authoritative", "restart-row-replay/pr-comment-marker-families")
add("adapter:github.issue-create-fork", "observe_issue", "claim_issue_for_management.fork_request", "adapter", "grantless-non-lifecycle", "fork-issue-create:v1;dedup=fork/original")
add("adapter:github.claim-label-add", "observe_issue", "claim_issue_for_management.add_claim_label", "adapter", "grantless-non-lifecycle", "claim-label/add/fkst-dev:claimed")
add("adapter:github.claim-label-remove", "observe_issue", "claim_issue_for_management.remove_claim_label", "adapter", "grantless-non-lifecycle", "claim-label/remove/fkst-dev:claimed")
add("adapter:github.issue-assign", "observe_issue", "claim_issue_for_management.assign", "adapter", "grantless-non-lifecycle", "assignee-claim/assign/self")
add("adapter:github.issue-unassign", "observe_issue", "claim_issue_for_management.unassign", "adapter", "grantless-non-lifecycle", "assignee-claim/unassign/self")
add("comment:issue:row-replay", "liveness_scan", "scan.restart_replay_issue_comments", "comment", "lifecycle-authoritative", "restart-row-replay/issue-comment-marker-families")
add("comment:pr:row-replay", "liveness_scan", "scan.restart_replay_pr_comments", "comment", "lifecycle-authoritative", "restart-row-replay/pr-comment-marker-families")
add("comment:issue:reconcile-blocked", "reconcile", "emit_blocked_reconcile.issue_comment", "comment", "lifecycle-authoritative", "state:v1/blocked+reconcile:v1;dedup=reconcile/comment")
add("label:issue:reconcile-blocked", "reconcile", "emit_blocked_reconcile.issue_label", "label", "lifecycle-authoritative", "state-label:blocked;dedup=reconcile/label")
add("comment:pr:reconcile-blocked", "reconcile", "emit_effects.pr_comment", "comment", "lifecycle-authoritative", "state:v1/blocked+review-reconcile|fix-reconcile:v1")
add("comment:issue:timeout-reconcile", "reconcile", "pipeline_timeout.comment", "comment", "lifecycle-authoritative", "state:v1/blocked+timeout-reconcile:v1;dedup=timeout-reconcile/comment")

return records
