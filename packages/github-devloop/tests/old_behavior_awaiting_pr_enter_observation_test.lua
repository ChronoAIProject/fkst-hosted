local config = require("devloop.config")
local devloop_logging = require("devloop.logging")
local devloop_state = require("devloop.state")
local entity_lib = require("devloop.entity")
local entity_read_mocks = require("tests.entity_read_mock_helpers")
local h = require("tests.devloop_helpers")
local m_builders = require("devloop.markers.builders")
local m_facts = require("devloop.markers.facts")
local observation_support = require("testkit.old_behavior_observation_support")
local testing = require("testkit.testing")

local t = h.t
local core = h.core
local awaiting_pr_replayer = require("core.awaiting_pr_replayer").install(core)
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
  symbol = "M.implementing_to_awaiting_pr_transition_status",
  ordinal = "versioned_transition_status:implementing->awaiting-pr",
}

local REPO = "owner/repo"
local ISSUE_NUMBER = 42
local PROPOSAL_ID = "github-devloop/issue/owner/repo/42"
local VERSION = "ready/consensus-github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z"
local OTHER_VERSION = "ready/consensus-github-devloop/issue/owner/repo/42/2026-06-02T01-02-03Z"
local BRANCH = "devloop-owner-repo-42-01HY"
local BASE_BRANCH = "dev"
local HEAD_SHA = "0123456789abcdef0123456789abcdef01234567"
local MERGE_COMMIT_SHA = "1111111111111111111111111111111111111111"

local FIXTURES = {
  {
    name = "pending-before-implementing",
    state = "ready",
    expected_probe = "pending",
    expected_status = "pending",
    expected_disposition = "retry-pending(from-state marker not yet visible)",
  },
  {
    name = "advanced-diverged-stale",
    state = "reviewing",
    expected_probe = "stale",
    expected_status = "stale",
    expected_disposition = "skip-advanced-or-diverged",
  },
  {
    name = "pr-delegation-missing",
    state = "implementing",
    expected_probe = "apply",
    expected_status = "skip-foreign",
    expected_disposition = "skip-foreign(pr-delegation-missing)",
  },
  {
    name = "pr-delegation-version",
    state = "implementing",
    delegation_version = OTHER_VERSION,
    expected_probe = "apply",
    expected_status = "skip-stale",
    expected_disposition = "skip-stale(pr-delegation-version)",
  },
  {
    name = "pr-delegation-child",
    state = "implementing",
    child_repo = "other/repo",
    expected_probe = "apply",
    expected_status = "skip-stale",
    expected_disposition = "skip-stale(pr-delegation-child)",
  },
  {
    name = "canonical-child-pr-merged-missing",
    state = "implementing",
    pr_number = 9706,
    child_state = "OPEN",
    expected_probe = "apply",
    expected_status = "skip-pending",
    expected_disposition = "skip-pending(canonical-child-pr-merged-missing)",
  },
  {
    name = "already-at-to-state",
    state = "awaiting-pr",
    pr_number = 9707,
    child_state = "MERGED",
    merged_at = "2026-06-03T02:05:04Z",
    expected_probe = "idempotent",
    expected_status = "idempotent",
    expected_disposition = "skip-idempotent(already at to_state)",
  },
  {
    name = "merged-delegated-pr-canonicalized",
    state = "implementing",
    pr_number = 9708,
    child_state = "MERGED",
    merged_at = "2026-06-03T02:05:04Z",
    expected_probe = "apply",
    expected_status = "apply",
    expected_disposition = "applied(merged-delegated-pr-canonicalized)",
    expected_effects = 2,
  },
}

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

local function fixture_comments(fixture)
  local comments = json_array({
    trusted_comment(core.state_marker(PROPOSAL_ID, fixture.state, VERSION)),
  })
  if fixture.name ~= "pr-delegation-missing" then
    table.insert(comments, trusted_comment(m_builders.pr_delegation_marker(
      PROPOSAL_ID,
      child_proposal_id(fixture),
      fixture.pr_number or 7,
      fixture.delegation_version or VERSION,
      "g1"
    ), "2026-06-03T01:01:00Z"))
  end
  return comments
end

local function child_comments(fixture)
  return json_array({
    trusted_comment(m_builders.pr_origin_marker(
      PROPOSAL_ID,
      ISSUE_NUMBER,
      BRANCH,
      VERSION,
      BASE_BRANCH
    ), "2026-06-03T01:02:00Z"),
  })
end

local function prepare_child_read_fakes(times)
  h.mock_bot_env()
  for _, fixture in ipairs(FIXTURES) do
    if fixture.child_state ~= nil then
      entity_read_mocks.mock_pr_read_forms(t, {
        repo = REPO,
        number = fixture.pr_number,
        comments = child_comments(fixture),
        head = BRANCH,
        head_sha = HEAD_SHA,
        merge_commit_sha = MERGE_COMMIT_SHA,
        state = fixture.child_state,
        merged_at = fixture.merged_at,
        base_branch = BASE_BRANCH,
        labels = {},
        status_check_rollup_json = "[]",
        register_all_views = true,
        times = times,
      })
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
      title = "Capture awaiting PR entry behavior",
      state = "OPEN",
      updated_at = "2026-06-03T02:03:04Z",
      dedup_key = "owner/repo#issue#42@2026-06-03T02:03:04Z/" .. fixture.name,
      source_ref = entity_lib.issue_source_ref(REPO, ISSUE_NUMBER),
    },
  }
end

local function writer_department(fixture)
  return {
    pipeline = function(event)
      local issue = copy_value(event.payload)
      issue.comments = fixture_comments(fixture)
      issue.labels = json_array({ "fkst-dev:enabled", "fkst-dev:" .. fixture.state })
      local state = devloop_state.current_state(issue.comments, PROPOSAL_ID)
      local delegation = m_facts.pr_delegation_fact(issue.comments, PROPOSAL_ID)
      return awaiting_pr_replayer.canonicalize_implementing_merged_delegated_pr(
        "observe_issue",
        issue,
        state,
        {
          proposal_id = PROPOSAL_ID,
          current = issue,
          current_issue = issue,
          fresh_current_state = state,
          ["pr-delegation"] = delegation,
        }
      )
    end,
  }
end

local function observe_writer(fixture, event)
  return observation_support.observe_department({
    config = config,
    devloop_logging = devloop_logging,
    devloop_state = devloop_state,
    dept = "observe_issue",
    from_state = "implementing",
    transition_kind = "versioned_transition_status",
    run = function()
      return testing.run_fake(writer_department(fixture), event)
    end,
    codex_runs_for_read = json_array(),
    write_mode = "real",
  })
end

local EFFECTS = {
  ["github-proxy.github_issue_comment_request"] = {
    effect_id = "comment:issue:awaiting-pr-state",
    sink_kind = "comment",
    authority_class = "lifecycle-authoritative",
  },
  ["github-proxy.github_issue_label_request"] = {
    effect_id = "label:issue:awaiting-pr-state",
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
      error("unclassified awaiting-pr-enter OLD raise: " .. tostring(raised.queue))
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

local function build_record(fixture, event, result, captured)
  local record = observation_support.build_record({
    t = t,
    dept = "observe_issue",
    event = event,
    result = result,
    captured = captured,
    owner = "github-devloop",
    site = SITE,
    observation_prefix = "writer:github-devloop:awaiting-pr-enter",
    observation_variant = fixture.name,
    transition_kind = "versioned_transition_status",
    source_state = function(probe) return probe.current.state end,
    outcome_status = function(probe, decision, apply)
      t.eq(probe.outcome, fixture.expected_probe, fixture.name .. ": real CAS regime")
      t.eq(decision.outcome, fixture.expected_disposition, fixture.name .. ": returned disposition")
      if fixture.expected_status == "apply" then
        t.eq(apply.to_state, "awaiting-pr", fixture.name .. ": apply target")
      else
        t.eq(apply, nil, fixture.name .. ": skipped writer has no apply")
      end
      return fixture.expected_status, fixture.expected_disposition, decision.outcome
    end,
    effects_from_raises = effects_from_raises,
    lineage = function(payload, probe)
      return {
        proposal_id = PROPOSAL_ID,
        issue_number = payload.number,
        pr_number = nullable(fixture.pr_number),
        delegation = fixture.name == "pr-delegation-missing" and JSON_NULL or "g1",
        state_version = probe.current.version,
        source_ref = copy_value(payload.source_ref),
      }
    end,
  })

  -- The production call's fourth argument is the target state's real version.
  record.typed_intent.generation_epoch.target_version = captured.probes[1].incoming_version
  record.old_inputs.target_version = captured.probes[1].incoming_version
  return record
end


local function capture_fixture(fixture)
  local event = fixture_event(fixture)
  local result, captured = observe_writer(fixture, event)
  t.eq(#captured.probes, 1, fixture.name .. ": one writer CAS probe")
  t.eq(#captured.decisions, 1, fixture.name .. ": one writer disposition")
  t.eq(captured.probes[1].incoming_version, VERSION, fixture.name .. ": target version is state.version")
  t.eq(captured.probes[1].target_version, nil, fixture.name .. ": no fifth resolver argument")
  t.eq(#captured.raises, fixture.expected_effects or 0, fixture.name .. ": captured raise count")
  t.eq(captured.liveness_read_count, 0, fixture.name .. ": writer never reads Codex liveness")
  local record = build_record(fixture, event, result, captured)
  t.eq(record.old_inputs.target_version, VERSION, fixture.name .. ": recorded target_version")
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
      error("OLD awaiting-pr-enter fixture " .. fixture.name .. " failed: " .. tostring(record), 0)
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
  test_awaiting_pr_enter_canonical_json_rejects_empty_array_object_drift = function()
    prepare_child_read_fakes(1)
    t.is_true(JSON_ARRAY_TAG ~= JSON_OBJECT_TAG, "json.decode preserves array versus object shape")
    local built = capture_fixture(FIXTURES[1])
    t.eq(canonical_json(built.old_outcome.emitted_effects), "[]", "empty effects retain array shape")

    local drifted = copy_value(built)
    drifted.old_outcome.emitted_effects = {}
    local root = "old_behavior_observations[awaiting-pr-enter][negative_control]"
    local difference = first_difference(drifted, built, root)
    t.is_true(canonical_json(drifted) ~= canonical_json(built), "empty array to object drift changes JSON")
    t.is_true(
      difference ~= nil
        and difference:find(root .. ".old_outcome.emitted_effects", 1, true) ~= nil,
      "empty-container drift diagnostic identifies emitted_effects: " .. tostring(difference)
    )
  end,

  test_awaiting_pr_enter_old_observations_are_hermetic_and_runtime_bound = function()
    prepare_child_read_fakes(2)
    local first = capture_all()
    local second = capture_all()
    local repeat_difference = first_difference(second, first, "old_behavior_observations[awaiting-pr-enter][repeat]")
    t.eq(repeat_difference, nil, "two identical fake-backed runs deep-equal")
    t.eq(canonical_json(second), canonical_json(first), "two identical fake-backed runs canonicalize equally")

    local expected = committed_records()
    local difference = first_difference(first, expected, "old_behavior_observations[awaiting-pr-enter]")
    if difference ~= nil or canonical_json(first) ~= canonical_json(expected) then
      error(
        "runtime-bound OLD awaiting-pr-enter observation differs at " .. tostring(difference or "canonical-json")
          .. "; runtime_records=" .. canonical_json(first),
        0
      )
    end
  end,
}
