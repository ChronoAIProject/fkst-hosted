local owner = "github-devloop-pr"
local records = {}
local effect_kinds = {}
for _, effect_kind in ipairs(require("devloop.restart_sinks").schema().effect_kinds) do
  effect_kinds[effect_kind] = effect_kind
end

local function add(id, department, site, effect_kind, authority_class, family)
  records[#records + 1] = {
    id = id,
    owner = owner,
    callsite = { department = department, site = site },
    effect_kind = effect_kind,
    authority_class = authority_class,
    dedup_marker_family = family,
  }
end

local function queue(department, name, authority_class, family)
  local qualified = name
  if qualified:find(".", 1, true) == nil then
    qualified = owner .. "." .. qualified
  end
  return add("queue:" .. qualified, department, "M.spec.produces:" .. name, "queue", authority_class, family)
end

queue("comment_handoff", "devloop_observe_pr", "grantless-telemetry", "observe-pr:v1/source-ref+dedup")
queue("comment_handoff", "devloop_merge_ready", "lifecycle-authoritative", "state:v1/merge-ready+review+head")
queue("comment_handoff", "devloop_fixing", "lifecycle-authoritative", "state:v1/fixing+head+version")
queue("comment_handoff", "devloop_reviewing", "lifecycle-authoritative", "state:v1/reviewing+head+version")
queue("fix", "devloop_review_meta", "lifecycle-authoritative", "state:v1/review-meta+review+head")
queue("fix", "devloop_fix_reconcile", "lifecycle-authoritative", "fix-reconcile:v1/proposal+round")
queue("fix", "github-devloop-decompose.devloop_decompose", "grantless-published-intent", "decompose.v1/proposal+attempt")
queue("liveness_scan", "devloop_observe_pr", "grantless-telemetry", "observe-pr:v1/source-ref+dedup")
queue("liveness_scan", "consensus.proposal", "lifecycle-authoritative", "consensus-proposal:v1/review-proposal+dedup")
queue("liveness_scan", "devloop_reviewing", "lifecycle-authoritative", "state:v1/reviewing+head+version")
queue("liveness_scan", "devloop_fixing", "lifecycle-authoritative", "state:v1/fixing+head+version")
queue("liveness_scan", "devloop_review_meta", "lifecycle-authoritative", "state:v1/review-meta+review+head")
queue("liveness_scan", "devloop_merge_ready", "lifecycle-authoritative", "state:v1/merge-ready+review+head")
queue("liveness_scan", "devloop_review_reconcile", "lifecycle-authoritative", "review-reconcile:v1/proposal+round")
queue("liveness_scan", "devloop_timeout_reconcile", "lifecycle-authoritative", "timeout-reconcile:v1/proposal+state+round")
queue("merge", "devloop_fix_reconcile", "lifecycle-authoritative", "fix-reconcile:v1/proposal+round")
queue("merge", "github-devloop-decompose.devloop_decompose", "grantless-published-intent", "decompose.v1/proposal+attempt")
queue("merge_queue", "devloop_fix_reconcile", "lifecycle-authoritative", "fix-reconcile:v1/proposal+round")
queue("merge_queue", "github-devloop-decompose.devloop_decompose", "grantless-published-intent", "decompose.v1/proposal+attempt")
queue("merge_queue", "devloop_merge_queue_tick", "lifecycle-authoritative", "merge-queue-tick:v1/repo+head")
queue("observe_pr", "github-devloop-decompose.devloop_decompose", "grantless-published-intent", "decompose.v1/proposal+attempt")
queue("observe_pr", "devloop_fix_reconcile", "lifecycle-authoritative", "fix-reconcile:v1/proposal+round")
queue("observe_pr", "devloop_fixing", "lifecycle-authoritative", "state:v1/fixing+head+version")
queue("observe_pr", "devloop_review_meta", "lifecycle-authoritative", "state:v1/review-meta+review+head")
queue("observe_pr", "devloop_merge_ready", "lifecycle-authoritative", "state:v1/merge-ready+review+head")
queue("observe_pr", "devloop_review_reconcile", "lifecycle-authoritative", "review-reconcile:v1/proposal+round")
queue("observe_pr", "devloop_timeout_reconcile", "lifecycle-authoritative", "timeout-reconcile:v1/proposal+state+round")
queue("review_loop", "consensus.proposal", "lifecycle-authoritative", "consensus-proposal:v1/review-proposal+dedup")
queue("review_loop", "devloop_review_reconcile", "lifecycle-authoritative", "review-reconcile:v1/proposal+round")
queue("review_pr", "consensus.proposal", "lifecycle-authoritative", "consensus-proposal:v1/review-proposal+dedup")
queue("review_result", "devloop_fix_reconcile", "lifecycle-authoritative", "fix-reconcile:v1/proposal+round")
queue("review_result", "github-devloop-decompose.devloop_decompose", "grantless-published-intent", "decompose.v1/proposal+attempt")
queue("review_result", "devloop_review_meta", "lifecycle-authoritative", "state:v1/review-meta+review+head")

add("adapter:structured-log", "all", "devloop_logging.log_line", "adapter", "grantless-telemetry", "structured-log:v1/dept+proposal+tag")
add("label:issue:comment-handoff-state", "comment_handoff", "emit_label_handoff.label", "label", "lifecycle-authoritative", "state-label:pr-state;dedup=comment-handoff/label")
add("comment:pr:fix-review-meta", "fix", "requests_review.raise_fix_review_meta.comment", "comment", "lifecycle-authoritative", "state:v1/review-meta;dedup=fix/review-meta/comment")
add("label:issue:fix-review-meta", "fix", "requests_review.raise_fix_review_meta.label", "label", "lifecycle-authoritative", "state-label:review-meta;dedup=fix/review-meta/label")
add("comment:pr:ci-repair-attempt", "fix", "ci_repair_attempts.raise_attempt_record", "comment", "lifecycle-authoritative", "ci-repair-attempt:v1;dedup=fix/ci-repair/attempt")
add("codex.dispatch:fix", "fix", "run_fix_codex.codex_dispatch", "codex", "lifecycle-authoritative", "codex-run:fix/proposal+work-unit+worker")
add("git.push:fix-branch", "fix", "publish_fix.push", effect_kinds.git, "lifecycle-authoritative", "git-push/fix-branch/proposal+head")
add("comment:issue:row-replay", "liveness_scan", "scan.restart_replay_issue_comments", "comment", "lifecycle-authoritative", "restart-row-replay/issue-comment-marker-families")
add("comment:pr:row-replay", "liveness_scan", "scan.restart_replay_pr_comments", "comment", "lifecycle-authoritative", "restart-row-replay/pr-comment-marker-families")
add("comment:pr:merge-fixing", "merge", "raise_fixing.comment", "comment", "lifecycle-authoritative", "state:v1/fixing+merge-gate:v1;dedup=merge/fixing/comment")
add("label:issue:merge-fixing", "merge", "raise_fixing.label", "label", "lifecycle-authoritative", "state-label:fixing;dedup=merge/fixing/label")
add("comment:pr:merge-head-reviewing", "merge", "raise_reviewing_for_current_head.comment", "comment", "lifecycle-authoritative", "state:v1/reviewing+head-change;dedup=merge/reviewing/comment")
add("label:issue:merge-head-reviewing", "merge", "raise_reviewing_for_current_head.label", "label", "lifecycle-authoritative", "state-label:reviewing;dedup=merge/reviewing/label")
add("comment:pr:merging-state", "merge", "write_merging_marker.comment", "comment", "lifecycle-authoritative", "state:v1/merging;dedup=merge/merging/head")
add("comment:pr:merged-state", "merge", "finalize_merged.comment", "comment", "lifecycle-authoritative", "state:v1/merged+autonomy-result:v1;dedup=merge/merged/comment")
add("comment:pr:merge-ci-wait", "merge", "merge_ci_wait.hold.comment", "comment", "lifecycle-authoritative", "merge-gate-wait:v1;dedup=merge/ci-wait/comment")
add("comment:pr:merge-queue-starvation", "merge", "process_merge_ready_locked.queue_starvation_comment", "comment", "lifecycle-authoritative", "queue-starvation-reconcile:v1;dedup=merge/queue-starvation")
add("adapter:github.pr-ready", "merge", "ensure_pr_ready_for_merge.ready", "adapter", "lifecycle-authoritative", "pr-ready/pr+head")
add("github.merge:verified-pr", "merge", "process_merge_ready_locked.verified_merge", "merge", "lifecycle-authoritative", "verified-merge/pr+reviewed-head+base")
add("comment:pr:merge-queue-executor", "merge_queue", "merge_executor.comment_sinks", "comment", "lifecycle-authoritative", "merge-executor/pr-comment-marker-families")
add("label:issue:merge-queue-executor", "merge_queue", "merge_executor.label_sinks", "label", "lifecycle-authoritative", "merge-executor/issue-label-families")
add("label:issue:observe-pr-state", "observe_pr", "maybe_pr_label_hint.label", "label", "lifecycle-authoritative", "state-label:pr-state;dedup=observe-pr/label")
add("comment:pr:operator-refusal", "observe_pr", "maybe_apply_rereview_command.refusal", "comment", "grantless-non-lifecycle", "operator-command:v1/refused;dedup=operator-command/refusal")
add("comment:pr:operator-rereview", "observe_pr", "maybe_apply_rereview_command.applied", "comment", "lifecycle-authoritative", "state:v1/reviewing+operator-command:v1/rereview")
add("comment:pr:observe-merge-gate-fix", "observe_pr", "raise_merge_gate_fix.comment", "comment", "lifecycle-authoritative", "state:v1/fixing+merge-gate:v1;dedup=observe-pr/fixing/comment")
add("label:issue:observe-merge-gate-fix", "observe_pr", "raise_merge_gate_fix.label", "label", "lifecycle-authoritative", "state-label:fixing;dedup=observe-pr/fixing/label")
add("comment:pr:observe-reviewing", "observe_pr", "build_reviewing_comment_request.comment", "comment", "lifecycle-authoritative", "state:v1/reviewing;dedup=observe-pr/reviewing/comment")
add("comment:pr:base-unmanaged", "observe_pr", "process_pr_event.base_unmanaged_comment", "comment", "lifecycle-authoritative", "state:v1/blocked+pr-base-unmanaged:v1")
add("comment:pr:row-replay", "observe_pr", "replay_pr_local_state.comments", "comment", "lifecycle-authoritative", "restart-row-replay/pr-comment-marker-families")
add("label:issue:row-replay", "observe_pr", "replay_pr_local_state.labels", "label", "lifecycle-authoritative", "restart-row-replay/issue-label-families")
add("comment:pr:reconcile-blocked", "reconcile", "emit_blocked_reconcile.pr_comment", "comment", "lifecycle-authoritative", "state:v1/blocked+review-reconcile|fix-reconcile:v1")
add("label:issue:reconcile-blocked", "reconcile", "emit_blocked_reconcile.issue_label", "label", "lifecycle-authoritative", "state-label:blocked;dedup=reconcile/label")
add("comment:issue:timeout-reconcile", "reconcile", "pipeline_timeout.issue_comment", "comment", "lifecycle-authoritative", "state:v1/blocked+timeout-reconcile:v1;dedup=timeout/issue")
add("comment:pr:timeout-reconcile", "reconcile", "pipeline_timeout.pr_comment", "comment", "lifecycle-authoritative", "state:v1/blocked+timeout-reconcile:v1;dedup=timeout/pr")
add("comment:issue:decompose-exhausted", "reconcile", "pipeline_timeout.decompose_exhausted_issue", "comment", "grantless-published-intent", "decompose-exhausted:v1/issue+attempt")
add("comment:pr:decompose-exhausted", "reconcile", "pipeline_timeout.decompose_exhausted_pr", "comment", "grantless-published-intent", "decompose-exhausted:v1/pr+attempt")
add("comment:pr:review-converge-round", "review_loop", "act.review_converge_round_comment", "comment", "lifecycle-authoritative", "review-converge-round:v1;dedup=review-loop/comment")
add("codex.dispatch:review-meta", "review_meta", "review_meta_codex_decision.codex_dispatch", "codex", "lifecycle-authoritative", "codex-run:review-meta/proposal+version+worker")
add("comment:pr:review-meta-result", "review_meta", "apply_review_meta_decision.comment", "comment", "lifecycle-authoritative", "state:v1/fixing|blocked+review-meta:v1")
add("label:issue:review-meta-result", "review_meta", "apply_review_meta_decision.label", "label", "lifecycle-authoritative", "state-label:fixing|blocked;dedup=review-meta/label")
add("comment:pr:review-result", "review_result", "apply_review_result.comment", "comment", "lifecycle-authoritative", "review-result:v1+state:v1;dedup=review-result/comment")
add("comment:pr:high-risk-review-evidence", "review_result", "apply_review_result.evidence_comment", "comment", "lifecycle-authoritative", "high-risk-review-evidence:v1;dedup=review-result/evidence")
add("label:issue:review-result", "review_result", "apply_review_result.label", "label", "lifecycle-authoritative", "state-label:merge-ready|fixing|review-meta")

return records
