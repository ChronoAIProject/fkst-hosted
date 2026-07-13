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
    id = "comment:issue:thinking-state",
    owner = "github-devloop",
    callsite = {
      department = "observe_issue",
      site = "process_issue_event.thinking_comment",
    },
    effect_kind = "comment",
    authority_class = "lifecycle-authoritative",
    dedup_marker_family = "state:v1/thinking;dedup=proposal/comment/thinking",
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

local owner = "github-devloop"

local department_names = {
  "comment_handoff",
  "consensus_result",
  "dead_letter",
  "execute_start",
  "implement",
  "liveness_scan",
  "loop",
  "observe_issue",
  "reconcile",
  "test_board_digest_probe",
  "test_cache_seed",
  "test_context_bundle_probe",
}

local request_surface_kinds = {
  ["github-proxy.github_issue_comment_request"] = "comment",
  ["github-proxy.github_issue_create_request"] = "adapter",
  ["github-proxy.github_issue_label_request"] = "label",
  ["github-proxy.github_pr_comment_request"] = "comment",
}

local queue_policies = {
  ["board_digest_probe"] = { "grantless-non-lifecycle", "test-probe/board-digest-request" },
  ["board_digest_result"] = { "grantless-non-lifecycle", "test-probe/board-digest-result" },
  ["cache_seed"] = { "grantless-non-lifecycle", "test-probe/cache-seed-request" },
  ["cache_seeded"] = { "grantless-non-lifecycle", "test-probe/cache-seed-result" },
  ["consensus.proposal"] = { "lifecycle-authoritative", "consensus-proposal:v1/proposal+dedup" },
  ["context_bundle_probe"] = { "grantless-non-lifecycle", "test-probe/context-bundle-request" },
  ["context_bundle_probe_result"] = { "grantless-non-lifecycle", "test-probe/context-bundle-result" },
  ["devloop_observe_issue"] = { "grantless-telemetry", "observe-issue:v1/source-ref+dedup" },
  ["devloop_ready"] = { "lifecycle-authoritative", "ready:v1/proposal+version" },
  ["devloop_reconcile"] = { "lifecycle-authoritative", "reconcile:v1/proposal+round" },
  ["devloop_timeout_reconcile"] = { "lifecycle-authoritative", "timeout-reconcile:v1/proposal+state+round" },
  ["github-devloop-decompose.devloop_decompose"] = { "grantless-published-intent", "decompose.v1/proposal+attempt" },
}

local semantic_specs = {
  { "adapter:structured-log", "all", "devloop_logging.log_line", "adapter", "grantless-telemetry", "structured-log:v1/dept+proposal+tag", "libraries/devloop/logging.lua", { "function C.log_line", "logging.log_line" } },
  { "comment:issue:consensus-result", "consensus_result", "raise_result_effects.result_comment", "comment", "lifecycle-authoritative", "state:v1+result:v1;dedup=proposal/comment/decision", "packages/github-devloop/departments/consensus_result/main.lua", { "build_result_comment_request", "comment_request" } },
  { "label:issue:consensus-result", "consensus_result", "raise_result_effects.result_label", "label", "lifecycle-authoritative", "state-label:ready;dedup=proposal/label/decision", "packages/github-devloop/departments/consensus_result/main.lua", { "build_result_label_request", "label_request" } },
  { "comment:issue:dependency-hold", "consensus_result", "raise_result_effects.dependency_hold_comment", "comment", "lifecycle-authoritative", "dependency-wait|dependency-cycle|dependency-unresolvable:v1;dedup=dependency/comment", "packages/github-devloop/departments/consensus_result/main.lua", { "build_dependency_hold_comment_request", "dependency_comment_request" } },
  { "label:issue:dependency-hold", "consensus_result", "raise_result_effects.dependency_hold_label", "label", "lifecycle-authoritative", "label:fkst-dev:blocked-on-dependency;dedup=dependency/label/hold", "packages/github-devloop/departments/consensus_result/main.lua", { "dependency_label_request", "_blocked_on_dependency_label" } },
  { "comment:issue:dependency-release", "consensus_result", "raise_result_effects.dependency_release_comment", "comment", "lifecycle-authoritative", "dependency-release:v1;dedup=dependency/comment/release", "packages/github-devloop/departments/consensus_result/main.lua", { "build_dependency_release_comment_request", "dependency_release_comment_request" } },
  { "comment:issue:thinking-state", "execute_start", "raise_execution_start.thinking_comment", "comment", "lifecycle-authoritative", "state:v1/thinking;dedup=execution-start/comment", "packages/github-devloop/departments/execute_start/main.lua", { "thinking_comment_request", "raise_execution_start" } },
  { "label:issue:thinking-state", "execute_start", "raise_execution_start.thinking_label", "label", "lifecycle-authoritative", "state-label:thinking;dedup=execution-start/label", "packages/github-devloop/departments/execute_start/main.lua", { "thinking_label_request", "raise_execution_start" } },
  { "comment:issue:implementation-failed", "implement", "raise_impl_failed.comment", "comment", "lifecycle-authoritative", "state:v1/impl-failed+impl-failure:v1;dedup=implement/comment/failure", "packages/github-devloop/departments/implement/main.lua", { "build_impl_failure_comment_request", "raise_impl_failed" } },
  { "label:issue:implementation-failed", "implement", "raise_impl_failed.label", "label", "lifecycle-authoritative", "state-label:impl-failed;dedup=implement/label/impl-failed", "packages/github-devloop/departments/implement/main.lua", { "build_impl_failed_label_request", "raise_impl_failed" } },
  { "comment:issue:implementation-start", "implement", "raise_implementing_state.comment", "comment", "lifecycle-authoritative", "state:v1/implementing+implement-attempt:v1;dedup=implement/comment/implementing-state", "packages/github-devloop/departments/implement/main.lua", { "build_implementing_state_comment_request", "raise_implementing_state" } },
  { "label:issue:implementation-start", "implement", "raise_implementing_state.label", "label", "lifecycle-authoritative", "state-label:implementing;dedup=implement/label/implementing", "packages/github-devloop/departments/implement/main.lua", { "build_implementing_label_request", "raise_implementing_state" } },
  { "comment:issue:implementation-progress", "implement", "raise_implementing.comment", "comment", "lifecycle-authoritative", "implementing:v1+implement-attempt:v1;dedup=implement/comment/implementing", "packages/github-devloop/departments/implement/main.lua", { "build_implementing_comment_request", "raise_implementing" } },
  { "comment:issue:implementation-attempt", "implement", "raise_implement_attempt.comment", "comment", "lifecycle-authoritative", "implement-attempt:v1;dedup=implement/comment/attempt", "packages/github-devloop/departments/implement/main.lua", { "build_implement_attempt_comment_request", "raise_implement_attempt" } },
  { "comment:issue:implementation-version-mismatch", "implement", "raise_implement_version_mismatch.comment", "comment", "lifecycle-authoritative", "implement-version-mismatch:v1;dedup=implement/comment/version-mismatch", "packages/github-devloop/departments/implement/main.lua", { "build_implement_version_mismatch_comment_request", "raise_implement_version_mismatch" } },
  { "comment:issue:dependency-canonicalization", "implement", "process_ready_event.dependency_canonicalization_comment", "comment", "lifecycle-authoritative", "state:v1/dependency_wait+ready-split-canonicalized:v1", "packages/github-devloop/departments/implement/main.lua", { "build_ready_split_canonicalized_comment_request", "dependency_wait" } },
  { "label:issue:dependency-canonicalization", "implement", "process_ready_event.dependency_hold_label", "label", "lifecycle-authoritative", "label:fkst-dev:blocked-on-dependency;dedup=dependency/label/hold", "packages/github-devloop/departments/implement/main.lua", { "_blocked_on_dependency_label", "dependency", "label", "hold" } },
  { "comment:pr:pr-child-open", "implement", "pr_child_handoff.child_start_comment", "comment", "lifecycle-authoritative", "state:v1/pr-open+pr-origin:v1+pr-link:v1;dedup=pr-delegation/pr-open", "packages/github-devloop/core/pr_delegation.lua", { "build_pr_open_comment_request", "pr_origin_marker" } },
  { "comment:issue:pr-delegation", "implement", "pr_child_handoff.issue_delegation_comment", "comment", "lifecycle-authoritative", "pr-delegation:v1;dedup=pr-delegation/issue", "packages/github-devloop/core/pr_delegation.lua", { "build_issue_delegation_comment_request", "pr_delegation_marker" } },
  { "comment:issue:awaiting-pr-state", "implement", "pr_child_handoff.parent_awaiting_comment", "comment", "lifecycle-authoritative", "state:v1/awaiting-pr+pr-delegation:v1;dedup=awaiting-pr", "packages/github-devloop/departments/implement/pr_child_handoff.lua", { "build_parent_awaiting_pr_comment_request", "awaiting-pr" } },
  { "label:issue:awaiting-pr-state", "implement", "pr_child_handoff.parent_awaiting_label", "label", "lifecycle-authoritative", "state-label:awaiting-pr;dedup=awaiting-pr/label", "packages/github-devloop/departments/implement/pr_child_handoff.lua", { "build_parent_awaiting_pr_label_request", "awaiting-pr" } },
  { "codex.dispatch:implement", "implement", "run_attempt.codex_dispatch", "codex", "lifecycle-authoritative", "codex-run:implement/proposal+dedup+worker", "packages/github-devloop/departments/implement/main.lua", { "workflow_codex.dispatch", "from_parts(\"implement\"" } },
  { "git.push:implementation-branch", "implement", "publish_implementation_branch.push", "git", "lifecycle-authoritative", "git-push/implementation-branch/proposal+version", "packages/github-devloop/departments/implement/main.lua", { "git_push_worktree_branch_update", "publish_implementation_branch" } },
  { "adapter:github.pr-create", "implement", "pr_child_handoff.create_pr", "adapter", "grantless-non-lifecycle", "pr-create/head+base+issue", "packages/github-devloop/core/pr_delegation.lua", { "gh_pr_create_body", "local function create_pr" } },
  { "comment:issue:duplicate-slice", "implement", "slice_gate.duplicate_comment", "comment", "grantless-non-lifecycle", "duplicate-slice:v1;dedup=implement/duplicate-slice", "packages/github-devloop/departments/implement/slice_gate.lua", { "duplicate-slice:v1", "duplicate_comment" } },
  { "label:issue:duplicate-slice", "implement", "slice_gate.duplicate_label", "label", "grantless-non-lifecycle", "label:fkst:duplicate-slice;dedup=implement/duplicate-slice/label", "packages/github-devloop/departments/implement/slice_gate.lua", { "fkst:duplicate-slice", "duplicate_label" } },
  { "adapter:github.issue-close-duplicate-slice", "implement", "slice_gate.close_duplicate", "adapter", "grantless-non-lifecycle", "issue-close/duplicate-slice", "packages/github-devloop/departments/implement/slice_gate.lua", { "gh_issue_close", "duplicate-slice-close-failed" } },
  { "comment:issue:duplicate-fork", "implement", "fork_gate.duplicate_comment", "comment", "grantless-non-lifecycle", "duplicate-fork:v1;dedup=implement/duplicate-fork", "packages/github-devloop/departments/implement/fork_gate.lua", { "duplicate-fork:v1", "duplicate_comment" } },
  { "label:issue:duplicate-fork", "implement", "fork_gate.duplicate_label", "label", "grantless-non-lifecycle", "label:fkst:duplicate-fork;dedup=implement/duplicate-fork/label", "packages/github-devloop/departments/implement/fork_gate.lua", { "fkst:duplicate-fork", "duplicate_label" } },
  { "adapter:github.issue-close-duplicate-fork", "implement", "fork_gate.close_duplicate", "adapter", "grantless-non-lifecycle", "issue-close/duplicate-fork", "packages/github-devloop/departments/implement/fork_gate.lua", { "gh_issue_close", "duplicate-fork-close-failed" } },
  { "comment:issue:converge-round", "loop", "act.converge_round_comment", "comment", "lifecycle-authoritative", "converge-round:v1;dedup=converge-round/comment", "packages/github-devloop/departments/loop/main.lua", { "build_converge_round_comment_request", "devloop_logging.log_raise(\"loop\", unresolved.proposal_id, \"github-proxy.github_issue_comment_request\", comment_request)" } },
  { "comment:issue:operator-refusal", "observe_issue", "operator_commands.refusal_comments", "comment", "grantless-non-lifecycle", "operator-command:v1/refused;dedup=operator-command/refusal", "packages/github-devloop/departments/observe_issue/main.lua", { "build_operator_issue_command_refusal_request", "devloop_logging.log_raise(\"observe_issue\", proposal_id, \"github-proxy.github_issue_comment_request\", refusal)" } },
  { "comment:issue:operator-rereview", "observe_issue", "maybe_apply_issue_rereview_command.applied_comment", "comment", "lifecycle-authoritative", "state:v1/thinking+operator-command:v1/rereview", "packages/github-devloop/departments/observe_issue/main.lua", { "maybe_apply_issue_rereview_command", "build_operator_issue_rereview_comment_request" } },
  { "comment:issue:operator-reimplement", "observe_issue", "maybe_apply_issue_reimplement_command.applied_comment", "comment", "lifecycle-authoritative", "operator-command:v1/reimplement;dedup=operator-command/reimplement", "packages/github-devloop/departments/observe_issue/main.lua", { "maybe_apply_issue_reimplement_command", "build_operator_issue_reimplement_comment_request" } },
  { "comment:issue:dependency-waiver", "observe_issue", "maybe_apply_issue_dependency_waiver_command.applied_comment", "comment", "lifecycle-authoritative", "dependency-waiver:v1+operator-command:v1;dedup=dependency-waiver", "packages/github-devloop/departments/observe_issue/main.lua", { "maybe_apply_issue_dependency_waiver_command", "dependency_waiver" } },
  { "label:issue:dependency-stale-clear", "observe_issue", "raise_stale_dependency_label_clear.label", "label", "lifecycle-authoritative", "label-remove:fkst-dev:blocked-on-dependency;dedup=dependency/label/clear-stale", "packages/github-devloop/departments/observe_issue/main.lua", { "raise_stale_dependency_label_clear", "devloop_logging.log_raise(\"observe_issue\", proposal_id, \"github-proxy.github_issue_label_request\", requests_labels.build_label_request" } },
  { "comment:issue:thinking-state", "observe_issue", "process_issue_event.thinking_comment", "comment", "lifecycle-authoritative", "state:v1/thinking;dedup=proposal/comment/thinking", "packages/github-devloop/departments/observe_issue/main.lua", { "build_observe_comment_request", "thinking" } },
  { "label:issue:thinking-state", "observe_issue", "process_issue_event.thinking_label", "label", "lifecycle-authoritative", "state-label:thinking;dedup=proposal/label/thinking", "packages/github-devloop/departments/observe_issue/main.lua", { "build_thinking_label_request", "thinking" } },
  { "comment:issue:row-replay", "observe_issue", "process_issue_event.restart_replay_comments", "comment", "lifecycle-authoritative", "restart-row-replay/issue-comment-marker-families", "packages/github-devloop/departments/observe_issue/main.lua", { "replayer.replay", "restart_transition_table" } },
  { "label:issue:row-replay", "observe_issue", "process_issue_event.restart_replay_labels", "label", "lifecycle-authoritative", "restart-row-replay/state-label-families", "packages/github-devloop/departments/observe_issue/main.lua", { "replayer.replay", "restart_transition_table" } },
  { "comment:pr:row-replay", "observe_issue", "process_issue_event.restart_replay_pr_comments", "comment", "lifecycle-authoritative", "restart-row-replay/pr-comment-marker-families", "packages/github-devloop/departments/observe_issue/main.lua", { "replayer.replay", "current_pr" } },
  { "adapter:github.issue-create-fork", "observe_issue", "claim_issue_for_management.fork_request", "adapter", "grantless-non-lifecycle", "fork-issue-create:v1;dedup=fork/original", "libraries/devloop/claims.lua", { "github-proxy.github_issue_create_request", "fork-raised" } },
  { "adapter:github.claim-label-add", "observe_issue", "claim_issue_for_management.add_claim_label", "adapter", "grantless-non-lifecycle", "claim-label/add/fkst-dev:claimed", "libraries/devloop/claims.lua", { "issue_add_label", "claim-won" } },
  { "adapter:github.claim-label-remove", "observe_issue", "claim_issue_for_management.remove_claim_label", "adapter", "grantless-non-lifecycle", "claim-label/remove/fkst-dev:claimed", "libraries/devloop/claims.lua", { "issue_remove_label", "claim-lost" } },
  { "adapter:github.issue-assign", "observe_issue", "claim_issue_for_management.assign", "adapter", "grantless-non-lifecycle", "assignee-claim/assign/self", "libraries/devloop/claims.lua", { "issue_assign", "claim-won" } },
  { "adapter:github.issue-unassign", "observe_issue", "claim_issue_for_management.unassign", "adapter", "grantless-non-lifecycle", "assignee-claim/unassign/self", "libraries/devloop/claims.lua", { "issue_unassign", "claim-lost" } },
  { "comment:issue:row-replay", "liveness_scan", "scan.restart_replay_issue_comments", "comment", "lifecycle-authoritative", "restart-row-replay/issue-comment-marker-families", "libraries/devloop/liveness/timeout.lua", { "replayer.replay_from_table_classified", "github-proxy.github_issue_comment_request" } },
  { "comment:pr:row-replay", "liveness_scan", "scan.restart_replay_pr_comments", "comment", "lifecycle-authoritative", "restart-row-replay/pr-comment-marker-families", "libraries/devloop/liveness/timeout.lua", { "replayer.replay_from_table_classified", "github-proxy.github_pr_comment_request" } },
  { "comment:issue:reconcile-blocked", "reconcile", "emit_blocked_reconcile.issue_comment", "comment", "lifecycle-authoritative", "state:v1/blocked+reconcile:v1;dedup=reconcile/comment", "packages/github-devloop/departments/reconcile/main.lua", { "emit_blocked_reconcile", "build_reconcile_comment_request" } },
  { "label:issue:reconcile-blocked", "reconcile", "emit_blocked_reconcile.issue_label", "label", "lifecycle-authoritative", "state-label:blocked;dedup=reconcile/label", "packages/github-devloop/departments/reconcile/main.lua", { "emit_blocked_reconcile", "build_reconcile_label_request" } },
  { "comment:pr:reconcile-blocked", "reconcile", "emit_effects.pr_comment", "comment", "lifecycle-authoritative", "state:v1/blocked+review-reconcile|fix-reconcile:v1", "packages/github-devloop/departments/reconcile/main.lua", { "github-proxy.github_pr_comment_request", "emit_effects" } },
  { "comment:issue:timeout-reconcile", "reconcile", "pipeline_timeout.comment", "comment", "lifecycle-authoritative", "state:v1/blocked+timeout-reconcile:v1;dedup=timeout-reconcile/comment", "packages/github-devloop/departments/reconcile/main.lua", { "pipeline_timeout", "build_timeout_reconcile_comment_request" } },
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
    local key = record.callsite.department .. "|" .. record.effect_kind
    semantic_surface[key] = true
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
  test_issue_restart_sink_inventory_matches_production_observations_symmetrically = function()
    local observed = production_observations()
    local snapshot = copy_value(sink_inventory)
    local extracted = restart_sinks.extract(owner, sink_inventory)

    t.eq(restart_sinks.assert_coverage(owner, extracted, observed, { symmetric = true }), true)
    assert_same_value(sink_inventory, snapshot)
    t.eq(#extracted, #observed)
  end,

  test_restart_sink_schema_is_closed = function()
    local schema = restart_sinks.schema()
    assert_same_value(schema, {
      authority_classes = {
        "grantless-non-lifecycle",
        "grantless-published-intent",
        "grantless-telemetry",
        "lifecycle-authoritative",
      },
      effect_kinds = {
        "adapter",
        "codex",
        "comment",
        "git",
        "label",
        "merge",
        "queue",
      },
    })
  end,

  test_restart_sink_extractor_is_ordered_immutable_and_deterministic = function()
    local inventory = {
      sink(),
      sink({
        id = "queue:github-devloop.devloop_observe_issue",
        callsite = {
          department = "liveness_scan",
          site = "scan.reobserve_issue",
        },
        effect_kind = "queue",
        authority_class = "grantless-telemetry",
        dedup_marker_family = "observe-issue:v1;dedup=liveness/observe",
      }),
    }
    local snapshot = copy_value(inventory)
    local extracted = restart_sinks.extract("github-devloop", inventory)

    assert_same_value(inventory, snapshot)
    assert_same_value(extracted, snapshot)
    t.is_true(extracted ~= inventory)
    t.is_true(extracted[1] ~= inventory[1])
    t.is_true(extracted[1].callsite ~= inventory[1].callsite)

    local repeated = restart_sinks.extract("github-devloop", inventory)
    assert_same_value(repeated, extracted)
    t.is_true(repeated ~= extracted)
    t.is_true(repeated[1] ~= extracted[1])
  end,

  test_restart_sink_coverage_fails_closed_on_unclassified_or_unobserved_sink = function()
    local authored = { sink() }
    local unclassified = sink({
      id = "git.push:implementation-branch",
      callsite = {
        department = "implement",
        site = "publish_implementation_branch.push",
      },
      effect_kind = "git",
      dedup_marker_family = "implementation-branch/proposal+version",
    })
    assert_fails(function()
      restart_sinks.assert_coverage("github-devloop", authored, { sink(), unclassified }, {
        symmetric = true,
      })
    end, "unclassified sink")

    assert_fails(function()
      restart_sinks.assert_coverage("github-devloop", { sink(), unclassified }, { sink() }, {
        symmetric = true,
      })
    end, "authored sink not observed")
  end,

  test_restart_sink_extractor_rejects_malformed_records_and_unknown_classes = function()
    assert_fails(function()
      restart_sinks.extract("", { sink() })
    end, "owner must be a non-empty string")
    assert_fails(function()
      restart_sinks.extract("github-devloop", "not-a-table")
    end, "inventory must be a table")
    assert_fails(function()
      restart_sinks.extract("github-devloop", { sink({ authority_class = "unknown" }) })
    end, "unknown authority_class")
    assert_fails(function()
      restart_sinks.extract("github-devloop", { sink({ id = "queue:github-devloop.devloop_ready", effect_kind = "comment" }) })
    end, "id does not match effect_kind")
    assert_fails(function()
      restart_sinks.extract("github-devloop", { sink(), sink() })
    end, "duplicate sink record")
    assert_fails(function()
      restart_sinks.extract("github-devloop", { sink({ callsite = { department = "observe_issue" } }) })
    end, "callsite")
    assert_fails(function()
      restart_sinks.extract("github-devloop", { sink({ dedup_marker_family = "" }) })
    end, "dedup_marker_family must be a non-empty string")
  end,
}
