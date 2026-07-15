local autonomy_ledger = require("devloop.autonomy_ledger")
local config = require("devloop.config")
local devloop_base = require("devloop.base")
local devloop_logging = require("devloop.logging")
local devloop_state = require("devloop.state")
local entity_lib = require("devloop.entity")
local entity_read_mocks = require("tests.entity_read_mock_helpers")
local github_commands = require("forge.github").new(function() end)
local h = require("tests.devloop_helpers")
local m_builders = require("devloop.markers.builders")
local liveness_scan = require("devloop.liveness_scan")
local observation_support = require("testkit.old_behavior_observation_support")
local replay_fields = require("devloop.replay_fields")
local replayer = require("devloop.replayer")
local testing = require("testkit.testing")
local transition_version = require("contract.transition_version")

local t = h.t
local core = h.core
local JSON_NULL = observation_support.JSON_NULL
local JSON_ARRAY_TAG = observation_support.JSON_ARRAY_TAG
local JSON_OBJECT_TAG = observation_support.JSON_OBJECT_TAG
local canonical_json = observation_support.canonical_json
local copy_value = observation_support.copy_value
local first_difference = observation_support.first_difference
local json_array = observation_support.json_array
local nullable = observation_support.nullable
local INVENTORY_PATH = "migration/restart-lifecycle.inventory.json"
local SITE = {
  path = "packages/github-devloop/core/awaiting_pr_replayer.lua",
  symbol = "M.replay_awaiting_pr_state",
  ordinal = "versioned_transition_status:awaiting-pr->next",
}

local REPO = "owner/repo"
local ISSUE_NUMBER = 42
local PROPOSAL_ID = "github-devloop/issue/owner/repo/42"
local VERSION = "ready/consensus-github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z"
local OTHER_VERSION = "ready/consensus-github-devloop/issue/owner/repo/42/2026-06-02T01-02-03Z"
local INTEGRATION_BRANCH = "integration/dev"
local UPSTREAM_BRANCH = "dev"
local OTHER_BASE_BRANCH = "integration/other"
local HEAD_SHA = "0123456789abcdef0123456789abcdef01234567"
local MERGE_COMMIT_SHA = "1111111111111111111111111111111111111111"
local ORIGINAL_BRANCH = devloop_base.implement_branch(
  REPO,
  ISSUE_NUMBER,
  core.implementation_base_version(VERSION)
)
local REPLACEMENT_VERSION = transition_version.reimplement_at(core.implementation_base_version(VERSION), 1)
local REPLACEMENT_BRANCH = devloop_base.implement_branch(REPO, ISSUE_NUMBER, REPLACEMENT_VERSION)

local FIXTURES = {
  {
    name = "pr-delegation-missing",
    delegation = false,
    expected_status = "skip-foreign",
    expected_disposition = "skip-foreign(pr-delegation-missing)",
  },
  {
    name = "pr-delegation-child",
    pr_number = 9800,
    child_repo = "other/repo",
    child_marker = false,
    expected_status = "skip-stale",
    expected_disposition = "skip-stale(pr-delegation-child)",
  },
  {
    name = "child-terminal-missing",
    pr_number = 9801,
    child_marker = false,
    expected_status = "skip-pending",
    expected_disposition = "skip-pending(child-terminal-missing)",
  },
  {
    name = "child-nonterminal",
    pr_number = 9802,
    child_state = "reviewing",
    expected_status = "skip-pending",
    expected_disposition = "skip-pending(child-nonterminal)",
  },
  {
    name = "child-state-lineage",
    pr_number = 9803,
    child_state = "blocked",
    child_version = OTHER_VERSION,
    expected_status = "skip-stale",
    expected_disposition = "skip-stale(child-state-lineage)",
  },
  {
    name = "child-branch-lineage",
    pr_number = 9804,
    child_state = "closed-unmerged",
    branch = "unexpected-child-branch",
    pr_state = "CLOSED",
    expected_status = "skip-stale",
    expected_disposition = "skip-stale(child-branch-lineage)",
  },
  {
    name = "canonical-child-pr-merged-missing",
    pr_number = 9805,
    child_state = "merged",
    pr_state = "OPEN",
    expected_status = "skip-pending",
    expected_disposition = "skip-pending(canonical-child-pr-merged-missing)",
  },
  {
    name = "pr-origin-rollup-lineage",
    pr_number = 9806,
    child_state = "merged",
    pr_state = "MERGED",
    base_branch = OTHER_BASE_BRANCH,
    expected_status = "skip-stale",
    expected_disposition = "skip-stale(pr-origin-rollup-lineage)",
  },
  {
    name = "merge-commit-missing",
    pr_number = 9807,
    child_state = "merged",
    pr_state = "MERGED",
    merge_commit_sha = "",
    expected_status = "skip-pending",
    expected_disposition = "skip-pending(merge-commit-missing)",
  },
  {
    name = "rollup-receipt-missing",
    pr_number = 9808,
    child_state = "merged",
    pr_state = "MERGED",
    rollup_receipt_missing = true,
    expected_status = "skip-pending",
    expected_disposition = "skip-pending(rollup-receipt-missing)",
  },
  {
    name = "child-pr-merged",
    pr_number = 9810,
    child_state = "merged",
    pr_state = "MERGED",
    base_branch = UPSTREAM_BRANCH,
    unified_branches = true,
    expected_probe = "apply",
    expected_target = "merged",
    expected_status = "apply",
    expected_disposition = "applied(child-pr-merged)",
    expected_effects = 2,
    expected_ledger_calls = 1,
  },
  {
    name = "child-pr-closed-unmerged",
    pr_number = 9811,
    child_state = "closed-unmerged",
    branch = ORIGINAL_BRANCH,
    pr_state = "CLOSED",
    expected_probe = "apply",
    expected_target = "ready",
    expected_status = "apply",
    expected_disposition = "applied(child-pr-closed-unmerged)",
    expected_effects = 2,
  },
  {
    name = "replacement-budget-exhausted",
    pr_number = 9812,
    child_state = "closed-unmerged",
    branch = REPLACEMENT_BRANCH,
    pr_state = "CLOSED",
    expected_probe = "apply",
    expected_target = "blocked",
    expected_status = "apply",
    expected_disposition = "applied(replacement-budget-exhausted)",
    expected_effects = 2,
  },
  {
    name = "child-pr-blocked",
    pr_number = 9813,
    child_state = "blocked",
    expected_probe = "apply",
    expected_target = "blocked",
    expected_status = "apply",
    expected_disposition = "applied(child-pr-blocked)",
    expected_effects = 2,
  },
}

-- The exact-target-marker branch after the CAS probe is target-parameterized,
-- but it is unreachable through observe_issue. Both production activation
-- payload shapes are pointer-only: github_poll's github_entity_changed payload
-- and liveness_scan_build_observe_payload omit comments. The department derives
-- state from its own issue-view comments; if an exact target marker is visible
-- in that snapshot, current_state selects the target and the awaiting-pr gate
-- prevents replay. Do not manufacture a state/comments skew in this inventory.

local function trusted_comment(body, created_at)
  return {
    body = body,
    author_login = "fkst-test-bot",
    created_at = created_at or "2026-06-03T01:00:00Z",
  }
end

local function child_proposal_id(fixture)
  return entity_lib.pr_proposal_id(fixture.child_repo or REPO, fixture.pr_number or 7)
end

local function next_state_version(fixture)
  if fixture.name == "child-pr-closed-unmerged" then
    return REPLACEMENT_VERSION
  end
  if fixture.name == "replacement-budget-exhausted" then
    return VERSION .. "/blocked/replacement-budget-exhausted"
  end
  if fixture.expected_target == "blocked" then
    return VERSION .. "/blocked/child-pr-blocked"
  end
  return VERSION
end

local function parent_comments(fixture)
  local comments = json_array({
    trusted_comment(core.state_marker(PROPOSAL_ID, "awaiting-pr", VERSION)),
  })
  if fixture.delegation ~= false then
    table.insert(comments, trusted_comment(m_builders.pr_delegation_marker(
      PROPOSAL_ID,
      child_proposal_id(fixture),
      fixture.pr_number or 7,
      fixture.delegation_version or VERSION,
      "g1"
    ), "2026-06-03T01:01:00Z"))
  end
  if fixture.exact_target_marker then
    table.insert(comments, trusted_comment(core.state_marker(
      PROPOSAL_ID,
      fixture.expected_target,
      next_state_version(fixture)
    ), "2026-06-03T01:02:00Z"))
  end
  return comments
end

local function child_comments(fixture)
  local comments = json_array({
    trusted_comment(m_builders.pr_origin_marker(
      PROPOSAL_ID,
      ISSUE_NUMBER,
      fixture.branch or ORIGINAL_BRANCH,
      VERSION,
      fixture.base_branch or INTEGRATION_BRANCH
    ), "2026-06-03T01:03:00Z"),
  })
  if fixture.child_marker ~= false then
    table.insert(comments, trusted_comment(core.state_marker(
      PROPOSAL_ID,
      fixture.child_state,
      fixture.child_version or VERSION
    ), "2026-06-03T01:04:00Z"))
  end
  return comments
end

local function reads_child(fixture)
  return fixture.pr_number ~= nil
end

local function prepare_external_fakes(times)
  h.mock_bot_env()
  for _, fixture in ipairs(FIXTURES) do
    if reads_child(fixture) then
      entity_read_mocks.mock_pr_read_forms(t, {
        repo = REPO,
        number = fixture.pr_number,
        comments = child_comments(fixture),
        head = fixture.branch or ORIGINAL_BRANCH,
        head_sha = HEAD_SHA,
        merge_commit_sha = fixture.merge_commit_sha ~= nil and fixture.merge_commit_sha or MERGE_COMMIT_SHA,
        state = fixture.pr_state or "OPEN",
        merged_at = fixture.pr_state == "MERGED" and "2026-06-03T02:05:04Z" or nil,
        base_branch = fixture.base_branch or INTEGRATION_BRANCH,
        labels = {},
        status_check_rollup_json = "[]",
        register_all_views = true,
        times = times,
      })
    end
    if fixture.rollup_receipt_missing then
      for _ = 1, times do
        t.mock_command(github_commands.pr_list_promotions_cmd(REPO, INTEGRATION_BRANCH, UPSTREAM_BRANCH), {
          stdout = "[[]]\n",
          stderr = "",
          exit_code = 0,
        })
      end
    end
    if fixture.name == "child-pr-merged" then
      for _ = 1, times do
        t.mock_command("gh issue close", {
          stdout = "closed\n",
          stderr = "",
          exit_code = 0,
        })
      end
    end
  end
end

local function fixture_event(fixture)
  return {
    queue = "github-proxy.github_entity_changed",
    payload = {
      schema = "github-proxy.v1",
      type = "issue",
      repo = REPO,
      number = ISSUE_NUMBER,
      title = "Capture awaiting PR exit behavior",
      body = "",
      state = "OPEN",
      updated_at = "2026-06-03T02:03:04Z",
      dedup_key = "owner/repo#issue#42@2026-06-03T02:03:04Z/" .. fixture.name,
      source_ref = entity_lib.issue_source_ref(REPO, ISSUE_NUMBER),
    },
  }
end

local function fake_autonomy_record(merge_ready, fixture)
  return {
    schema = "github-devloop.autonomy-result.v1",
    proposal_id = tostring(merge_ready.proposal_id),
    repo = REPO,
    issue_number = tostring(ISSUE_NUMBER),
    pr_number = tostring(merge_ready.pr_number),
    version = tostring(merge_ready.version),
    head_sha = tostring(merge_ready.reviewed_head_sha),
    merged_at = fixture.pr_state == "MERGED" and "2026-06-03T02:05:04Z" or nil,
    task_class = "L1",
    human_touch_count = 0,
    pre_merge_ci = "pass",
    rounds = 0,
    retry_count = 0,
    codex_calls = nil,
    gates = {
      human_touch = "pass",
      pre_merge_ci = "pass",
      evidence_manifest = "pending",
      post_merge_probe = "pending",
      no_revert_reopen = "pending",
      cost_budget = "pending",
    },
    valid_autonomous_merge = "pending",
  }
end

local function writer_department(fixture)
  return {
    pipeline = function(event)
      local issue = copy_value(event.payload)
      issue.comments = parent_comments(fixture)
      issue.labels = json_array({ "fkst-dev:enabled", "fkst-dev:awaiting-pr" })
      local state = devloop_state.current_state(issue.comments, PROPOSAL_ID)
      local row = replay_fields.restart_transition_row(core.restart_transition_table(), "awaiting-pr")
      return replayer.replay_from_table(core, "observe_issue", issue, state, row, {
        proposal_id = PROPOSAL_ID,
        current = issue,
        current_issue = issue,
        fresh_current_state = state,
      })
    end,
  }
end

local function observe_writer(fixture, event)
  local original_branch_config = config.branch_config
  local original_autonomy_result_record = autonomy_ledger.autonomy_result_record
  local ledger_calls = 0
  config.branch_config = function()
    return {
      integration = fixture.unified_branches and UPSTREAM_BRANCH or INTEGRATION_BRANCH,
      upstream = UPSTREAM_BRANCH,
    }
  end
  autonomy_ledger.autonomy_result_record = function(_, repo, issue_number, merge_ready)
    ledger_calls = ledger_calls + 1
    t.eq(repo, REPO, fixture.name .. ": AVM fixture repo")
    t.eq(issue_number, ISSUE_NUMBER, fixture.name .. ": AVM fixture issue")
    return fake_autonomy_record(merge_ready, fixture)
  end
  local ok, result, captured = pcall(function()
    return observation_support.observe_department({
      config = config,
      devloop_logging = devloop_logging,
      devloop_state = devloop_state,
      dept = "observe_issue",
      from_state = "awaiting-pr",
      transition_kind = "versioned_transition_status",
      run = function()
        return testing.run_fake(writer_department(fixture), event)
      end,
      codex_runs_for_read = json_array(),
      write_mode = "real",
    })
  end)
  autonomy_ledger.autonomy_result_record = original_autonomy_result_record
  config.branch_config = original_branch_config
  if not ok then
    error(result, 0)
  end
  return result, captured, ledger_calls
end

local EFFECTS = {
  ["github-proxy.github_issue_comment_request"] = {
    effect_id = "comment:issue:awaiting-pr-terminal",
    sink_kind = "comment",
    authority_class = "lifecycle-authoritative",
  },
  ["github-proxy.github_issue_label_request"] = {
    effect_id = "label:issue:awaiting-pr-terminal",
    sink_kind = "label",
    authority_class = "lifecycle-authoritative",
  },
}

local function effects_from_raises(raises)
  local effects = json_array()
  local writes = json_array()
  for ordinal, raised in ipairs(raises) do
    local shape = EFFECTS[raised.queue]
    if shape == nil then
      error("unclassified awaiting-pr-exit OLD raise: " .. tostring(raised.queue))
    end
    table.insert(effects, {
      effect_id = shape.effect_id,
      sink_kind = shape.sink_kind,
      authority_class = shape.authority_class,
      ordinal = ordinal,
    })
    table.insert(writes, {
      effect_id = shape.effect_id,
      queue = raised.queue,
      payload = copy_value(raised.payload),
    })
  end
  return effects, writes
end

local function lineage(payload, fixture)
  return {
    proposal_id = PROPOSAL_ID,
    issue_number = payload.number,
    pr_number = nullable(fixture.pr_number),
    delegation = fixture.delegation == false and JSON_NULL or "g1",
    state_version = VERSION,
    child_state = nullable(fixture.child_state),
    source_ref = copy_value(payload.source_ref),
  }
end

local function build_guard_record(fixture, event, result, captured)
  t.eq(#captured.probes, 0, fixture.name .. ": guard returns before writer CAS")
  t.eq(#captured.decisions, 1, fixture.name .. ": guard logs one disposition")
  t.eq(#captured.applies, 0, fixture.name .. ": guard logs no apply")
  t.eq(#captured.raises, 0, fixture.name .. ": guard logs no raise")
  t.eq(#result.raises, 0, fixture.name .. ": guard emits no effect")
  local decision = captured.decisions[1]
  t.eq(decision.outcome, fixture.expected_disposition, fixture.name .. ": exact guard disposition")
  return {
    schema = "restart-old-behavior-observation.v2",
    observation_id = table.concat({
      "writer:github-devloop:awaiting-pr-exit",
      fixture.name,
      decision.to_state,
      fixture.expected_status,
      fixture.expected_disposition,
      "none",
    }, "/"),
    owner = "github-devloop",
    site = copy_value(SITE),
    boundary = "writer",
    typed_intent = {
      kind = "versioned_transition_status",
      source_state = "awaiting-pr",
      source_boundary = JSON_NULL,
      target = decision.to_state,
      cause_schema_id = event.payload.schema,
      generation_epoch = {
        current_version = VERSION,
        incoming_version = VERSION,
        target_version = JSON_NULL,
      },
      lineage = lineage(event.payload, fixture),
    },
    old_inputs = {
      current_fact = {
        state = "awaiting-pr",
        version = VERSION,
        stage_rank = devloop_state.stage_rank("awaiting-pr"),
      },
      caller_from_states = json_array({ "awaiting-pr" }),
      incoming_version = VERSION,
      target_version = JSON_NULL,
      handoff_reference = JSON_NULL,
    },
    old_outcome = {
      status = fixture.expected_status,
      reason_code = fixture.expected_disposition,
      cas_outcome = decision.outcome,
      emitted_effects = json_array(),
      observable_writes = json_array(),
      handoff_direct_lookup_count = captured.handoff_direct_lookup_count,
      timeout_evidence_source = JSON_NULL,
    },
    evidence_refs = json_array({
      {
        kind = "runtime-cas-decision",
        ref = "devloop.logging.log_cas_decision:" .. decision.outcome,
      },
      {
        kind = "runtime-event-source",
        ref = tostring(event.payload.source_ref and event.payload.source_ref.ref),
      },
    }),
  }
end

local function build_cas_record(fixture, event, result, captured)
  local record = observation_support.build_record({
    t = t,
    dept = "observe_issue",
    event = event,
    result = result,
    captured = captured,
    owner = "github-devloop",
    site = SITE,
    observation_prefix = "writer:github-devloop:awaiting-pr-exit",
    observation_variant = fixture.name,
    transition_kind = "versioned_transition_status",
    source_state = function(probe) return probe.current.state end,
    outcome_status = function(probe, decision, apply)
      t.eq(probe.outcome, fixture.expected_probe, fixture.name .. ": real CAS regime")
      t.eq(probe.to_state, fixture.expected_target, fixture.name .. ": runtime target")
      t.eq(decision.outcome, fixture.expected_disposition, fixture.name .. ": returned disposition")
      if fixture.expected_status == "apply" then
        t.eq(apply.to_state, fixture.expected_target, fixture.name .. ": apply target")
        t.eq(apply.version, next_state_version(fixture), fixture.name .. ": apply version")
      else
        t.eq(apply, nil, fixture.name .. ": skipped writer has no apply")
      end
      return fixture.expected_status, fixture.expected_disposition, decision.outcome
    end,
    effects_from_raises = effects_from_raises,
    lineage = function(payload) return lineage(payload, fixture) end,
  })

  -- The production four-argument CAS uses state.version as its target-version fact.
  record.typed_intent.generation_epoch.target_version = captured.probes[1].incoming_version
  record.old_inputs.target_version = captured.probes[1].incoming_version
  return record
end

local function capture_fixture(fixture)
  local event = fixture_event(fixture)
  local result, captured, ledger_calls = observe_writer(fixture, event)
  t.eq(captured.liveness_read_count, 0, fixture.name .. ": writer never reads Codex liveness")
  t.eq(ledger_calls, fixture.expected_ledger_calls or 0, fixture.name .. ": fake AVM ledger call count")
  t.eq(#captured.raises, fixture.expected_effects or 0, fixture.name .. ": captured raise count")
  local record
  if fixture.expected_probe == nil then
    record = build_guard_record(fixture, event, result, captured)
  else
    t.eq(#captured.probes, 1, fixture.name .. ": one writer CAS probe")
    t.eq(captured.probes[1].incoming_version, VERSION, fixture.name .. ": target version is state.version")
    t.eq(captured.probes[1].target_version, nil, fixture.name .. ": no fifth resolver argument")
    record = build_cas_record(fixture, event, result, captured)
    t.eq(record.old_inputs.target_version, VERSION, fixture.name .. ": recorded target_version")
  end
  t.eq(record.old_outcome.status, fixture.expected_status, fixture.name .. ": recorded status")
  t.eq(record.old_outcome.reason_code, fixture.expected_disposition, fixture.name .. ": recorded disposition")
  t.eq(#record.old_outcome.emitted_effects, fixture.expected_effects or 0, fixture.name .. ": effect count")
  return record
end

local function capture_all()
  local records = json_array()
  for _, fixture in ipairs(FIXTURES) do
    local ok, record = pcall(capture_fixture, fixture)
    if not ok then
      error("OLD awaiting-pr-exit fixture " .. fixture.name .. " failed: " .. tostring(record), 0)
    end
    table.insert(records, record)
  end
  table.sort(records, function(left, right)
    return left.observation_id < right.observation_id
  end)
  return records
end

local function is_target_record(record)
  local site = type(record) == "table" and record.site or nil
  return type(site) == "table"
    and site.path == SITE.path
    and site.symbol == SITE.symbol
    and site.ordinal == SITE.ordinal
end

local function committed_records()
  local inventory = json.decode(file.read(INVENTORY_PATH))
  local selected = json_array()
  for _, record in ipairs(inventory.old_behavior_observations or {}) do
    if is_target_record(record) then
      table.insert(selected, record)
    end
  end
  table.sort(selected, function(left, right)
    return tostring(left.observation_id) < tostring(right.observation_id)
  end)
  return selected
end

return {
  test_awaiting_pr_exit_canonical_json_rejects_empty_array_object_drift = function()
    prepare_external_fakes(1)
    t.is_true(JSON_ARRAY_TAG ~= JSON_OBJECT_TAG, "json.decode preserves array versus object shape")
    local built = capture_fixture(FIXTURES[1])
    t.eq(canonical_json(built.old_outcome.emitted_effects), "[]", "empty effects retain array shape")

    local drifted = copy_value(built)
    drifted.old_outcome.emitted_effects = {}
    local root = "old_behavior_observations[awaiting-pr-exit][negative_control]"
    local difference = first_difference(drifted, built, root)
    t.is_true(canonical_json(drifted) ~= canonical_json(built), "empty array to object drift changes JSON")
    t.is_true(
      difference ~= nil
        and difference:find(root .. ".old_outcome.emitted_effects", 1, true) ~= nil,
      "empty-container drift diagnostic identifies emitted_effects: " .. tostring(difference)
    )
  end,

  test_awaiting_pr_exit_real_cas_reachability_is_apply_only = function()
    local current = { state = "awaiting-pr", version = VERSION }
    for _, target in ipairs({ "merged", "ready", "blocked" }) do
      t.eq(
        devloop_state.versioned_transition_status(current, { "awaiting-pr" }, target, current.version),
        "apply",
        target .. ": guarded same-version CAS is apply"
      )
      t.is_true(target ~= current.state, target .. ": already-at-target idempotence is unreachable")
    end
    t.eq(
      devloop_state.versioned_transition_status({ state = "ready", version = VERSION }, { "awaiting-pr" }, "merged", VERSION),
      "pending",
      "pending requires a state rejected by the pre-CAS awaiting-pr guard"
    )
    t.eq(
      devloop_state.versioned_transition_status({ state = "awaiting-pr", version = VERSION }, { "awaiting-pr" }, "merged", OTHER_VERSION),
      "stale",
      "stale requires an incoming version different from the production state.version argument"
    )
  end,

  test_awaiting_pr_exit_exact_target_marker_skip_is_not_production_reachable = function()
    local apply_derivations = 0
    for _, fixture in ipairs(FIXTURES) do
      if fixture.expected_status == "apply" then
        apply_derivations = apply_derivations + 1
        local with_target_marker = copy_value(fixture)
        with_target_marker.exact_target_marker = true
        local derived = devloop_state.current_state(parent_comments(with_target_marker), PROPOSAL_ID)
        t.eq(derived.state, fixture.expected_target, fixture.name .. ": exact marker advances derived state")
        t.eq(derived.version, next_state_version(fixture), fixture.name .. ": exact marker advances derived version")
      end
      t.eq(fixture.exact_target_marker, nil, fixture.name .. ": observation fixture has no manufactured marker skew")
    end
    t.eq(apply_derivations, 4, "all four apply derivations are covered by the reachability ruling")

    local poll_payload = fixture_event(FIXTURES[1]).payload
    t.eq(poll_payload.comments, nil, "github_entity_changed observation payload is pointer-only")
    local redrive_payload = liveness_scan.liveness_scan_build_observe_payload(REPO, {
      number = ISSUE_NUMBER,
      title = "Capture awaiting PR exit behavior",
      state = "OPEN",
      updated_at = "2026-06-03T02:03:04Z",
    }, "issue", "tick")
    t.eq(redrive_payload.comments, nil, "devloop_observe_issue production payload is pointer-only")
  end,

  test_awaiting_pr_exit_removed_guards_are_not_presented_to_the_handler = function()
    h.mock_bot_env()
    local fixture = FIXTURES[1]
    local event = fixture_event(fixture)
    local issue = copy_value(event.payload)
    issue.comments = parent_comments(fixture)
    local row = replay_fields.restart_transition_row(core.restart_transition_table(), "awaiting-pr")

    local original = core.replayer_registry["awaiting-pr"]
    local handler_calls = 0
    core.replayer_registry["awaiting-pr"] = function(...)
      handler_calls = handler_calls + 1
      return original(...)
    end
    local ok, result = pcall(function()
      return testing.run_fake({
        pipeline = function()
          return replayer.replay_from_table(core, "observe_issue", issue, {
            state = "implementing",
            version = VERSION,
          }, row, { proposal_id = PROPOSAL_ID, current = issue })
        end,
      }, event)
    end)
    core.replayer_registry["awaiting-pr"] = original
    if not ok then error(result, 0) end
    t.eq(handler_calls, 0, "outer production dispatch rejects foreign state before awaiting-pr handler")
    t.eq(#result.raises, 0, "foreign state dispatch emits no effect")

    local mismatch = {
      name = "non-production-pr-delegation-version",
      delegation_version = OTHER_VERSION,
    }
    local mismatch_event = fixture_event(mismatch)
    local mismatch_issue = copy_value(mismatch_event.payload)
    mismatch_issue.comments = parent_comments(mismatch)
    local mismatch_state = devloop_state.current_state(mismatch_issue.comments, PROPOSAL_ID)
    local gathered = replayer.gather_replay_required_facts(core, row, mismatch_issue, mismatch_state, {
      proposal_id = PROPOSAL_ID,
      current = mismatch_issue,
      current_issue = mismatch_issue,
      fresh_current_state = mismatch_state,
    })
    t.eq(gathered["pr-delegation"], nil, "production fact gathering filters version-mismatched delegation")
    t.eq(gathered["child-state"], nil, "filtered delegation cannot manufacture a child-state fact")

    local _, captured = observe_writer(mismatch, mismatch_event)
    t.eq(#captured.decisions, 1, "version mismatch reaches one production-path disposition")
    t.eq(
      captured.decisions[1].outcome,
      "skip-foreign(pr-delegation-missing)",
      "version mismatch is production-visible only as missing delegation"
    )
  end,

  test_awaiting_pr_exit_old_observations_are_hermetic_and_runtime_bound = function()
    prepare_external_fakes(2)
    local first = capture_all()
    local second = capture_all()
    t.eq(#first, 14, "complete production-reachable awaiting-pr-exit observation count")
    local repeat_difference = first_difference(second, first, "old_behavior_observations[awaiting-pr-exit][repeat]")
    t.eq(repeat_difference, nil, "two identical fake-backed runs deep-equal")
    t.eq(canonical_json(second), canonical_json(first), "two identical fake-backed runs canonicalize equally")

    local expected = committed_records()
    local difference = first_difference(first, expected, "old_behavior_observations[awaiting-pr-exit]")
    if difference ~= nil or canonical_json(first) ~= canonical_json(expected) then
      error(
        "runtime-bound OLD awaiting-pr-exit observation differs at " .. tostring(difference or "canonical-json")
          .. "; runtime_records=" .. canonical_json(first),
        0
      )
    end
  end,
}
