local h = require("tests.devloop_core_helpers")
local sink_inventory = require("core.restart.sink_inventory")
local restart_sinks = require("devloop.restart_sinks")

local t = h.t

local function copy_value(value)
  if type(value) ~= "table" then
    return value
  end
  local out = {}
  for key, nested in pairs(value) do
    out[key] = copy_value(nested)
  end
  return out
end

local function assert_same_value(actual, expected)
  if type(expected) ~= "table" then
    t.eq(actual, expected)
    return
  end
  t.eq(type(actual), "table")
  local actual_count = 0
  for _ in pairs(actual) do
    actual_count = actual_count + 1
  end
  local expected_count = 0
  for key, nested in pairs(expected) do
    expected_count = expected_count + 1
    assert_same_value(actual[key], nested)
  end
  t.eq(actual_count, expected_count)
end

local function sink(overrides)
  local record = {
    id = "comment:pr:review-result",
    owner = "github-devloop-pr",
    callsite = {
      department = "review_result",
      site = "apply_review_result.comment",
    },
    effect_kind = "comment",
    authority_class = "lifecycle-authoritative",
    dedup_marker_family = "state:v1/review-result;dedup=review-result/comment",
  }
  for key, value in pairs(overrides or {}) do
    record[key] = value
  end
  return record
end

local function assert_fails(fn, needle)
  local ok, err = pcall(fn)
  t.eq(ok, false)
  t.is_true(tostring(err):find(needle, 1, true) ~= nil, tostring(err))
end

local owner = "github-devloop-pr"

local department_names = {
  "comment_handoff",
  "dead_letter",
  "fix",
  "liveness_scan",
  "merge",
  "merge_queue",
  "observe_pr",
  "reconcile",
  "review_loop",
  "review_meta",
  "review_pr",
  "review_result",
}

local request_surface_kinds = {
  ["github-proxy.github_issue_comment_request"] = "comment",
  ["github-proxy.github_issue_label_request"] = "label",
  ["github-proxy.github_pr_comment_request"] = "comment",
}

local queue_policies = {
  ["consensus.proposal"] = { "lifecycle-authoritative", "consensus-proposal:v1/review-proposal+dedup" },
  ["devloop_fix_reconcile"] = { "lifecycle-authoritative", "fix-reconcile:v1/proposal+round" },
  ["devloop_fixing"] = { "lifecycle-authoritative", "state:v1/fixing+head+version" },
  ["devloop_merge_queue_tick"] = { "lifecycle-authoritative", "merge-queue-tick:v1/repo+head" },
  ["devloop_merge_ready"] = { "lifecycle-authoritative", "state:v1/merge-ready+review+head" },
  ["devloop_observe_pr"] = { "grantless-telemetry", "observe-pr:v1/source-ref+dedup" },
  ["devloop_review_meta"] = { "lifecycle-authoritative", "state:v1/review-meta+review+head" },
  ["devloop_review_reconcile"] = { "lifecycle-authoritative", "review-reconcile:v1/proposal+round" },
  ["devloop_reviewing"] = { "lifecycle-authoritative", "state:v1/reviewing+head+version" },
  ["devloop_timeout_reconcile"] = { "lifecycle-authoritative", "timeout-reconcile:v1/proposal+state+round" },
  ["github-devloop-decompose.devloop_decompose"] = { "grantless-published-intent", "decompose.v1/proposal+attempt" },
}

local semantic_specs = {
  { "adapter:structured-log", "all", "devloop_logging.log_line", "adapter", "grantless-telemetry", "structured-log:v1/dept+proposal+tag", "libraries/devloop/logging.lua", { "function C.log_line", "logging.log_line" } },
  { "label:issue:comment-handoff-state", "comment_handoff", "emit_label_handoff.label", "label", "lifecycle-authoritative", "state-label:pr-state;dedup=comment-handoff/label", "packages/github-devloop-pr/departments/comment_handoff/main.lua", { "build_reconcile_pr_state_label_request", "github-proxy.github_issue_label_request" } },

  { "comment:pr:fix-review-meta", "fix", "requests_review.raise_fix_review_meta.comment", "comment", "lifecycle-authoritative", "state:v1/review-meta;dedup=fix/review-meta/comment", "libraries/devloop/requests/review.lua", { "function C.raise_fix_review_meta", "github-proxy.github_pr_comment_request" } },
  { "label:issue:fix-review-meta", "fix", "requests_review.raise_fix_review_meta.label", "label", "lifecycle-authoritative", "state-label:review-meta;dedup=fix/review-meta/label", "libraries/devloop/requests/review.lua", { "function C.raise_fix_review_meta", "github-proxy.github_issue_label_request" } },
  { "comment:pr:ci-repair-attempt", "fix", "ci_repair_attempts.raise_attempt_record", "comment", "lifecycle-authoritative", "ci-repair-attempt:v1;dedup=fix/ci-repair/attempt", "packages/github-devloop-pr/core/ci_repair_attempts.lua", { "function C.raise_attempt_record", "github-proxy.github_pr_comment_request" } },
  { "codex.dispatch:fix", "fix", "run_fix_codex.codex_dispatch", "codex", "lifecycle-authoritative", "codex-run:fix/proposal+work-unit+worker", "packages/github-devloop-pr/departments/fix/main.lua", { "workflow_codex.dispatch", "from_parts(\"fix\"" } },
  { "git.push:fix-branch", "fix", "publish_fix.push", "git", "lifecycle-authoritative", "git-push/fix-branch/proposal+head", "packages/github-devloop-pr/departments/fix/main.lua", { "git_push_ref_update", "local push =" } },

  { "comment:issue:row-replay", "liveness_scan", "scan.restart_replay_issue_comments", "comment", "lifecycle-authoritative", "restart-row-replay/issue-comment-marker-families", "libraries/devloop/liveness/timeout.lua", { "replayer.replay_from_table_classified", "github-proxy.github_issue_comment_request" } },
  { "comment:pr:row-replay", "liveness_scan", "scan.restart_replay_pr_comments", "comment", "lifecycle-authoritative", "restart-row-replay/pr-comment-marker-families", "libraries/devloop/liveness/timeout.lua", { "replayer.replay_from_table_classified", "github-proxy.github_pr_comment_request" } },

  { "comment:pr:merge-fixing", "merge", "raise_fixing.comment", "comment", "lifecycle-authoritative", "state:v1/fixing+merge-gate:v1;dedup=merge/fixing/comment", "packages/github-devloop-pr/core/merge_executor.lua", { "local function raise_fixing", "github-proxy.github_pr_comment_request" } },
  { "label:issue:merge-fixing", "merge", "raise_fixing.label", "label", "lifecycle-authoritative", "state-label:fixing;dedup=merge/fixing/label", "packages/github-devloop-pr/core/merge_executor.lua", { "local function raise_fixing", "github-proxy.github_issue_label_request" } },
  { "comment:pr:merge-head-reviewing", "merge", "raise_reviewing_for_current_head.comment", "comment", "lifecycle-authoritative", "state:v1/reviewing+head-change;dedup=merge/reviewing/comment", "packages/github-devloop-pr/core/merge_executor.lua", { "build_merge_head_reviewing_comment_request", "github-proxy.github_pr_comment_request" } },
  { "label:issue:merge-head-reviewing", "merge", "raise_reviewing_for_current_head.label", "label", "lifecycle-authoritative", "state-label:reviewing;dedup=merge/reviewing/label", "packages/github-devloop-pr/core/merge_executor.lua", { "build_merge_head_reviewing_label_request", "github-proxy.github_issue_label_request" } },
  { "comment:pr:merging-state", "merge", "write_merging_marker.comment", "comment", "lifecycle-authoritative", "state:v1/merging;dedup=merge/merging/head", "packages/github-devloop-pr/core/merge_executor.lua", { "local function write_merging_marker", "core.gh_pr_comment" } },
  { "comment:pr:merged-state", "merge", "finalize_merged.comment", "comment", "lifecycle-authoritative", "state:v1/merged+autonomy-result:v1;dedup=merge/merged/comment", "packages/github-devloop-pr/core/merge_executor.lua", { "local function finalize_merged", "github-proxy.github_pr_comment_request" } },
  { "comment:pr:merge-ci-wait", "merge", "merge_ci_wait.hold.comment", "comment", "lifecycle-authoritative", "merge-gate-wait:v1;dedup=merge/ci-wait/comment", "packages/github-devloop-pr/core/merge_ci_wait.lua", { "build_merge_gate_wait_comment_request", "github-proxy.github_pr_comment_request" } },
  { "comment:pr:merge-queue-starvation", "merge", "process_merge_ready_locked.queue_starvation_comment", "comment", "lifecycle-authoritative", "queue-starvation-reconcile:v1;dedup=merge/queue-starvation", "packages/github-devloop-pr/core/merge_executor.lua", { "build_queue_starvation_reconcile_comment_request", "github-proxy.github_pr_comment_request" } },
  { "adapter:github.pr-ready", "merge", "ensure_pr_ready_for_merge.ready", "adapter", "lifecycle-authoritative", "pr-ready/pr+head", "packages/github-devloop-pr/core/merge_executor.lua", { "local function ensure_pr_ready_for_merge", "core.gh_pr_ready" } },
  { "github.merge:verified-pr", "merge", "process_merge_ready_locked.verified_merge", "merge", "lifecycle-authoritative", "verified-merge/pr+reviewed-head+base", "packages/github-devloop-pr/core/merge_executor.lua", { "core.run_verified_pr_merge", "before_merge" } },
  { "comment:pr:merge-queue-executor", "merge_queue", "merge_executor.comment_sinks", "comment", "lifecycle-authoritative", "merge-executor/pr-comment-marker-families", "packages/github-devloop-pr/departments/merge_queue/main.lua", { "merge_executor.process_merge_queue_tick", "github-proxy.github_pr_comment_request" } },
  { "label:issue:merge-queue-executor", "merge_queue", "merge_executor.label_sinks", "label", "lifecycle-authoritative", "merge-executor/issue-label-families", "packages/github-devloop-pr/departments/merge_queue/main.lua", { "merge_executor.process_merge_queue_tick", "github-proxy.github_issue_label_request" } },

  { "label:issue:observe-pr-state", "observe_pr", "maybe_pr_label_hint.label", "label", "lifecycle-authoritative", "state-label:pr-state;dedup=observe-pr/label", "packages/github-devloop-pr/departments/observe_pr/main.lua", { "build_reconcile_pr_state_label_request", "github-proxy.github_issue_label_request" } },
  { "comment:pr:operator-refusal", "observe_pr", "maybe_apply_rereview_command.refusal", "comment", "grantless-non-lifecycle", "operator-command:v1/refused;dedup=operator-command/refusal", "packages/github-devloop-pr/departments/observe_pr/main.lua", { "build_operator_command_refusal_request", "github-proxy.github_pr_comment_request" } },
  { "comment:pr:operator-rereview", "observe_pr", "maybe_apply_rereview_command.applied", "comment", "lifecycle-authoritative", "state:v1/reviewing+operator-command:v1/rereview", "packages/github-devloop-pr/departments/observe_pr/main.lua", { "build_operator_rereview_comment_request", "github-proxy.github_pr_comment_request" } },
  { "comment:pr:observe-merge-gate-fix", "observe_pr", "raise_merge_gate_fix.comment", "comment", "lifecycle-authoritative", "state:v1/fixing+merge-gate:v1;dedup=observe-pr/fixing/comment", "packages/github-devloop-pr/departments/observe_pr/main.lua", { "build_merge_gate_fix_comment_request", "github-proxy.github_pr_comment_request" } },
  { "label:issue:observe-merge-gate-fix", "observe_pr", "raise_merge_gate_fix.label", "label", "lifecycle-authoritative", "state-label:fixing;dedup=observe-pr/fixing/label", "packages/github-devloop-pr/departments/observe_pr/main.lua", { "requests_labels.build_state_label_request", "github-proxy.github_issue_label_request" } },
  { "comment:pr:observe-reviewing", "observe_pr", "build_reviewing_comment_request.comment", "comment", "lifecycle-authoritative", "state:v1/reviewing;dedup=observe-pr/reviewing/comment", "packages/github-devloop-pr/departments/observe_pr/main.lua", { "local function build_reviewing_comment_request", "github-proxy.github_pr_comment_request" } },
  { "comment:pr:base-unmanaged", "observe_pr", "process_pr_event.base_unmanaged_comment", "comment", "lifecycle-authoritative", "state:v1/blocked+pr-base-unmanaged:v1", "packages/github-devloop-pr/departments/observe_pr/main.lua", { "build_pr_base_unmanaged_comment_request", "github-proxy.github_pr_comment_request" } },
  { "comment:pr:row-replay", "observe_pr", "replay_pr_local_state.comments", "comment", "lifecycle-authoritative", "restart-row-replay/pr-comment-marker-families", "packages/github-devloop-pr/departments/observe_pr/main.lua", { "replayer.replay_from_table", "restart_transition_table" } },
  { "label:issue:row-replay", "observe_pr", "replay_pr_local_state.labels", "label", "lifecycle-authoritative", "restart-row-replay/issue-label-families", "packages/github-devloop-pr/departments/observe_pr/main.lua", { "replayer.replay_from_table", "restart_transition_table" } },

  { "comment:pr:reconcile-blocked", "reconcile", "emit_blocked_reconcile.pr_comment", "comment", "lifecycle-authoritative", "state:v1/blocked+review-reconcile|fix-reconcile:v1", "packages/github-devloop-pr/departments/reconcile/main.lua", { "emit_blocked_reconcile", "github-proxy.github_pr_comment_request" } },
  { "label:issue:reconcile-blocked", "reconcile", "emit_blocked_reconcile.issue_label", "label", "lifecycle-authoritative", "state-label:blocked;dedup=reconcile/label", "packages/github-devloop-pr/departments/reconcile/main.lua", { "emit_blocked_reconcile", "github-proxy.github_issue_label_request" } },
  { "comment:issue:timeout-reconcile", "reconcile", "pipeline_timeout.issue_comment", "comment", "lifecycle-authoritative", "state:v1/blocked+timeout-reconcile:v1;dedup=timeout/issue", "packages/github-devloop-pr/departments/reconcile/main.lua", { "build_timeout_reconcile_comment_request", "github-proxy.github_issue_comment_request" } },
  { "comment:pr:timeout-reconcile", "reconcile", "pipeline_timeout.pr_comment", "comment", "lifecycle-authoritative", "state:v1/blocked+timeout-reconcile:v1;dedup=timeout/pr", "packages/github-devloop-pr/departments/reconcile/main.lua", { "build_timeout_reconcile_pr_comment_request", "github-proxy.github_pr_comment_request" } },
  { "comment:issue:decompose-exhausted", "reconcile", "pipeline_timeout.decompose_exhausted_issue", "comment", "grantless-published-intent", "decompose-exhausted:v1/issue+attempt", "packages/github-devloop-pr/departments/reconcile/main.lua", { "build_decompose_exhausted_comment_request", "github-proxy.github_issue_comment_request" } },
  { "comment:pr:decompose-exhausted", "reconcile", "pipeline_timeout.decompose_exhausted_pr", "comment", "grantless-published-intent", "decompose-exhausted:v1/pr+attempt", "packages/github-devloop-pr/departments/reconcile/main.lua", { "build_decompose_exhausted_comment_request", "github-proxy.github_pr_comment_request" } },

  { "comment:pr:review-converge-round", "review_loop", "act.review_converge_round_comment", "comment", "lifecycle-authoritative", "review-converge-round:v1;dedup=review-loop/comment", "packages/github-devloop-pr/departments/review_loop/main.lua", { "build_review_converge_round_comment_request", "github-proxy.github_pr_comment_request" } },
  { "codex.dispatch:review-meta", "review_meta", "review_meta_codex_decision.codex_dispatch", "codex", "lifecycle-authoritative", "codex-run:review-meta/proposal+version+worker", "packages/github-devloop-pr/departments/review_meta/main.lua", { "workflow_codex.dispatch", "from_parts(\"review-meta\"" } },
  { "comment:pr:review-meta-result", "review_meta", "apply_review_meta_decision.comment", "comment", "lifecycle-authoritative", "state:v1/fixing|blocked+review-meta:v1", "packages/github-devloop-pr/departments/review_meta/main.lua", { "build_review_meta_comment_request", "github-proxy.github_pr_comment_request" } },
  { "label:issue:review-meta-result", "review_meta", "apply_review_meta_decision.label", "label", "lifecycle-authoritative", "state-label:fixing|blocked;dedup=review-meta/label", "packages/github-devloop-pr/departments/review_meta/main.lua", { "build_review_meta_label_request", "github-proxy.github_issue_label_request" } },
  { "comment:pr:review-result", "review_result", "apply_review_result.comment", "comment", "lifecycle-authoritative", "review-result:v1+state:v1;dedup=review-result/comment", "packages/github-devloop-pr/departments/review_result/main.lua", { "build_review_result_comment_request", "github-proxy.github_pr_comment_request" } },
  { "comment:pr:high-risk-review-evidence", "review_result", "apply_review_result.evidence_comment", "comment", "lifecycle-authoritative", "high-risk-review-evidence:v1;dedup=review-result/evidence", "packages/github-devloop-pr/departments/review_result/main.lua", { "build_high_risk_review_evidence_comment_request", "evidence_request" } },
  { "label:issue:review-result", "review_result", "apply_review_result.label", "label", "lifecycle-authoritative", "state-label:merge-ready|fixing|review-meta", "packages/github-devloop-pr/departments/review_result/main.lua", { "build_review_result_label_request", "github-proxy.github_issue_label_request" } },
}

local observation_deferrals = {
  "live GitHub, git, and codex branch execution is represented by the explicit production callsite evidence above; no synthetic partial external snapshot is treated as a full observation",
}

local source_cache = {}

local function source_text(path)
  if source_cache[path] == nil then
    source_cache[path] = file.read(path)
  end
  return source_cache[path]
end

local function semantic_observations()
  local observed = {}
  for _, spec in ipairs(semantic_specs) do
    local source = source_text(spec[7])
    for _, token in ipairs(spec[8]) do
      t.is_true(source:find(token, 1, true) ~= nil, spec[7] .. " lacks production sink evidence " .. token)
    end
    table.insert(observed, {
      id = spec[1],
      owner = owner,
      callsite = { department = spec[2], site = spec[3] },
      effect_kind = spec[4],
      authority_class = spec[5],
      dedup_marker_family = spec[6],
    })
  end
  for _, reason in ipairs(observation_deferrals) do
    log.info("github-devloop-pr restart sink conformance deferred runtime branch capture: " .. reason)
  end
  return observed
end

local function qualified_queue(queue_name)
  if queue_name:find(".", 1, true) ~= nil then
    return queue_name
  end
  return owner .. "." .. queue_name
end

local function queue_observations(semantic)
  local observed = {}
  local semantic_surface = {}
  for _, record in ipairs(semantic) do
    semantic_surface[record.callsite.department .. "|" .. record.effect_kind] = true
  end
  for _, department_name in ipairs(department_names) do
    local department = require("departments." .. department_name .. ".main")
    for _, queue_name in ipairs(department.spec.produces or {}) do
      local request_kind = request_surface_kinds[queue_name]
      if request_kind ~= nil then
        t.eq(semantic_surface[department_name .. "|" .. request_kind], true)
      else
        local policy = queue_policies[queue_name]
        if policy == nil then
          error("restart sink conformance: unclassified sink " .. department_name .. "|queue|" .. queue_name)
        end
        table.insert(observed, {
          id = "queue:" .. qualified_queue(queue_name),
          owner = owner,
          callsite = {
            department = department_name,
            site = "M.spec.produces:" .. queue_name,
          },
          effect_kind = "queue",
          authority_class = policy[1],
          dedup_marker_family = policy[2],
        })
      end
    end
  end
  return observed
end

local function production_observations()
  local observed = semantic_observations()
  for _, record in ipairs(queue_observations(observed)) do
    table.insert(observed, record)
  end
  return observed
end

return {
  test_pr_restart_sink_inventory_matches_production_observations_symmetrically = function()
    local observed = production_observations()
    local snapshot = copy_value(sink_inventory)
    local extracted = restart_sinks.extract(owner, sink_inventory)

    t.eq(restart_sinks.assert_coverage(owner, extracted, observed, { symmetric = true }), true)
    assert_same_value(sink_inventory, snapshot)
    t.eq(#extracted, #observed)
  end,

  test_pr_restart_sink_observation_deferrals_are_explicit = function()
    t.is_true(#observation_deferrals > 0)
    for _, reason in ipairs(observation_deferrals) do
      t.eq(type(reason), "string")
      t.is_true(reason ~= "")
    end
  end,

  test_pr_restart_sink_coverage_fails_closed_on_unclassified_or_unobserved_sink = function()
    local authored = { sink() }
    local unclassified = sink({
      id = "github.merge:verified-pr",
      callsite = {
        department = "merge",
        site = "process_merge_ready_locked.verified_merge",
      },
      effect_kind = "merge",
      dedup_marker_family = "verified-merge/pr+reviewed-head+base",
    })
    assert_fails(function()
      restart_sinks.assert_coverage(owner, authored, { sink(), unclassified }, { symmetric = true })
    end, "unclassified sink")

    assert_fails(function()
      restart_sinks.assert_coverage(owner, { sink(), unclassified }, { sink() }, { symmetric = true })
    end, "authored sink not observed")
  end,

  test_pr_restart_sink_extractor_rejects_unknown_class_and_wrong_owner = function()
    assert_fails(function()
      restart_sinks.extract(owner, { sink({ authority_class = "unknown" }) })
    end, "unknown authority_class")
    assert_fails(function()
      restart_sinks.extract(owner, { sink({ owner = "github-devloop" }) })
    end, "sink owner must match extractor owner")
  end,
}
