local devloop_base = require("devloop.base")
local devloop_logging = require("devloop.logging")
local devloop_state = require("devloop.state")
local entity_read_mocks = require("tests.entity_read_mock_helpers")
local h = require("tests.devloop_helpers")
local m_builders = require("devloop.markers.builders")
local observation_support = require("testkit.old_behavior_observation_support")
local payloads_builders = require("devloop.payloads.builders")
local testing = require("testkit.testing")
local observe_pr_department = require("departments.observe_pr.main")

local t = h.t
local core = h.core
local JSON_NULL = observation_support.JSON_NULL
local JSON_ARRAY_TAG = observation_support.JSON_ARRAY_TAG
local JSON_OBJECT_TAG = observation_support.JSON_OBJECT_TAG
local canonical_json = observation_support.canonical_json
local copy_value = observation_support.copy_value
local first_difference = observation_support.first_difference
local json_array = observation_support.json_array
local INVENTORY_PATH = "migration/restart-lifecycle.inventory.json"
local SITE = {
  path = "packages/github-devloop-pr/core/pr_review_replayer.lua",
  symbol = "replay_review_result",
  ordinal = "build_devloop_merge_ready_payload",
}

local REPO = "owner/repo"
local PR_NUMBER = 7
local BRANCH = "devloop-owner-repo-42-01HY"
local BASE_BRANCH = "dev"
local HEAD_SHA = "def456"
local ISSUE_PROPOSAL_ID = "github-devloop/issue/owner/repo/42"
local ISSUE_VERSION = "ready/consensus-github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z"

local FIXTURES = {
  {
    name = "issue-backed-review-result-replay",
    proposal_id = ISSUE_PROPOSAL_ID,
    issue_number = 42,
    version = ISSUE_VERSION,
    tick = "2026-06-04T01:02:03Z",
  },
}

local function trusted_comment(body, id)
  return {
    id = id,
    body = body,
    author_login = "fkst-test-bot",
    created_at = "2026-06-03T01:00:00Z",
  }
end

local function review_identity(fixture)
  local review_id = devloop_base.pr_review_proposal_id(REPO, PR_NUMBER, fixture.version, HEAD_SHA)
  return review_id, "consensus:" .. review_id .. "/review"
end

local function fixture_comments(fixture)
  local review_id, review_dedup = review_identity(fixture)
  local comments = json_array()
  table.insert(comments, trusted_comment(m_builders.pr_origin_marker(
    fixture.proposal_id,
    tostring(fixture.issue_number),
    BRANCH,
    fixture.version,
    BASE_BRANCH
  ), "IC_origin_" .. fixture.name))
  table.insert(comments, trusted_comment(
    core.state_marker(fixture.proposal_id, "reviewing", fixture.version),
    "IC_state_" .. fixture.name
  ))
  table.insert(comments, trusted_comment(m_builders.review_result_marker(
    review_id,
    fixture.proposal_id,
    "approve",
    review_dedup
  ), "IC_review_result_" .. fixture.name))
  table.insert(comments, trusted_comment(m_builders.merge_ready_marker(
    fixture.proposal_id,
    PR_NUMBER,
    fixture.version,
    review_id,
    review_dedup,
    HEAD_SHA
  ), "IC_merge_ready_" .. fixture.name))
  return comments
end

local function event_for(fixture)
  return {
    queue = "github-proxy.github_entity_changed",
    ts = fixture.tick,
    now_seconds = 1784048400,
    payload = {
      schema = "github-proxy.v1",
      type = "pr",
      repo = REPO,
      number = PR_NUMBER,
      state = "OPEN",
      updated_at = fixture.tick,
      dedup_key = "owner/repo#pr#7@" .. fixture.tick,
      source_ref = { kind = "external", ref = "owner/repo#pr/7" },
    },
  }
end

local function prepare_fixture(fixture)
  h.mock_bot_env()
  entity_read_mocks.mock_pr_read_forms(t, {
    repo = REPO,
    number = PR_NUMBER,
    comments = fixture_comments(fixture),
    head = BRANCH,
    head_sha = HEAD_SHA,
    state = "OPEN",
    base_branch = BASE_BRANCH,
    labels = { "fkst-dev:reviewing" },
    updated_at = fixture.tick,
    times = 2,
  })
  entity_read_mocks.mock_issue_view_selector(t, {
    repo = REPO,
    number = fixture.issue_number,
    assignees = { "fkst-test-bot" },
    author_login = "fkst-test-bot",
  }, "assignees,author", 3)
end

local function capture_runtime(fixture)
  local event = event_for(fixture)
  prepare_fixture(fixture)
  local constructor_calls = json_array()
  local decisions = json_array()
  local original_builder = payloads_builders.build_devloop_merge_ready_payload
  local original_decision = devloop_logging.log_cas_decision

  payloads_builders.build_devloop_merge_ready_payload = function(proposal_id, pr_number, version, review_fact, source_ref)
    local payload = original_builder(proposal_id, pr_number, version, review_fact, source_ref)
    table.insert(constructor_calls, {
      review_fact = copy_value(review_fact),
      payload = payload,
    })
    return payload
  end
  devloop_logging.log_cas_decision = function(dept, proposal_id, current, from_state, to_state, outcome, reason)
    if dept == "observe_pr" and outcome == "applied(replay)" and to_state == "merge-ready" then
      table.insert(decisions, {
        proposal_id = proposal_id,
        current = copy_value(current),
        from_state = from_state,
        to_state = to_state,
        outcome = outcome,
        reason = reason,
      })
    end
    return original_decision(dept, proposal_id, current, from_state, to_state, outcome, reason)
  end

  local ok, result = pcall(testing.run_fake, observe_pr_department, event)
  devloop_logging.log_cas_decision = original_decision
  payloads_builders.build_devloop_merge_ready_payload = original_builder
  if not ok then
    error(result, 0)
  end

  t.eq(#constructor_calls, 1, fixture.name .. ": real observe_pr dispatch calls merge-ready constructor once")
  t.eq(#decisions, 1, fixture.name .. ": real replayer reaches trusted approval replay")
  local merge_ready_raises = json_array()
  for _, raised in ipairs(result.raises) do
    if raised.queue == "devloop_merge_ready" then
      table.insert(merge_ready_raises, copy_value(raised))
    end
  end
  t.eq(#merge_ready_raises, 1, fixture.name .. ": real replayer emits one devloop_merge_ready payload")
  local constructor = constructor_calls[1]
  constructor.payload = copy_value(constructor.payload)
  t.eq(
    canonical_json(merge_ready_raises[1].payload),
    canonical_json(constructor.payload),
    fixture.name .. ": emitted payload exactly matches the constructor result"
  )
  return event, constructor, decisions[1], merge_ready_raises[1]
end

local function build_record(fixture)
  local event, constructor, decision, merge_ready_raise = capture_runtime(fixture)
  local payload = constructor.payload
  return {
    schema = "restart-old-behavior-observation.v2",
    observation_id = "ctor:github-devloop-pr:review-replayer-merge-ready/" .. fixture.name,
    owner = "github-devloop-pr",
    site = copy_value(SITE),
    boundary = "writer",
    typed_intent = {
      kind = "direct_constructor",
      source_state = "reviewing",
      source_boundary = event.queue,
      target = "devloop_merge_ready",
      cause_schema_id = event.payload.schema,
      generation_epoch = {
        current_version = decision.current.version,
        source_version = fixture.version,
        payload_version = payload.version,
      },
      lineage = {
        proposal_id = payload.proposal_id,
        issue_number = fixture.issue_number,
        pr_number = payload.pr_number,
        review_proposal_id = payload.review_proposal_id,
        review_dedup_key = payload.review_dedup_key,
        reviewed_head_sha = payload.reviewed_head_sha,
        current_head_sha = constructor.review_fact.current_head_sha,
        source_ref = copy_value(payload.source_ref),
      },
    },
    old_inputs = {
      current_fact = {
        state = decision.current.state,
        version = decision.current.version,
        stage_rank = decision.current.stage_rank,
      },
      caller_from_states = json_array({ "reviewing" }),
      incoming_version = fixture.version,
      target_version = payload.version,
      handoff_reference = JSON_NULL,
    },
    old_outcome = {
      status = "constructed",
      reason_code = fixture.name,
      cas_outcome = "not-applicable-direct-constructor",
      emitted_effects = json_array({
        {
          effect_id = "queue:devloop_merge_ready",
          sink_kind = "queue",
          authority_class = "lifecycle-authoritative",
          ordinal = 1,
        },
      }),
      observable_writes = json_array({
        {
          effect_id = "queue:devloop_merge_ready",
          queue = merge_ready_raise.queue,
          payload = copy_value(merge_ready_raise.payload),
        },
      }),
      handoff_direct_lookup_count = 0,
      timeout_evidence_source = JSON_NULL,
    },
    evidence_refs = json_array({
      {
        kind = "runtime-constructor-call",
        ref = "devloop.payloads.builders.build_devloop_merge_ready_payload",
      },
      {
        kind = "runtime-event-source",
        ref = event.payload.source_ref.ref,
      },
      {
        kind = "production-callsite",
        ref = "packages/github-devloop-pr/core/pr_review_replayer.lua:180",
      },
    }),
  }
end


local function capture_records()
  local records = json_array()
  for _, fixture in ipairs(FIXTURES) do
    table.insert(records, build_record(fixture))
  end
  table.sort(records, function(left, right)
    return left.observation_id < right.observation_id
  end)
  return records
end

local function committed_records()
  local inventory = json.decode(file.read(INVENTORY_PATH))
  local selected = json_array()
  for _, record in ipairs(inventory.old_behavior_observations or {}) do
    local site = type(record) == "table" and record.site or nil
    if type(site) == "table"
      and site.path == SITE.path
      and site.symbol == SITE.symbol
      and site.ordinal == SITE.ordinal then
      table.insert(selected, record)
    end
  end
  table.sort(selected, function(left, right)
    return tostring(left.observation_id) < tostring(right.observation_id)
  end)
  return selected
end

return {
  test_review_replayer_merge_ready_canonical_json_rejects_empty_array_object_drift = function()
    t.is_true(JSON_ARRAY_TAG ~= JSON_OBJECT_TAG, "json.decode preserves array versus object shape")
    local expected = { emitted_effects = json_array() }
    local drifted = copy_value(expected)
    drifted.emitted_effects = {}
    local root = "old_behavior_observations[review-replayer-merge-ready][negative_control]"
    local difference = first_difference(drifted, expected, root)
    t.eq(canonical_json(expected.emitted_effects), "[]", "empty effects retain array shape")
    t.is_true(canonical_json(drifted) ~= canonical_json(expected), "empty array to object drift changes JSON")
    t.is_true(difference ~= nil and difference:find(root .. ".emitted_effects", 1, true) ~= nil)
  end,

  test_review_replayer_merge_ready_old_observations_are_real_dispatch_runtime_bound_and_bidirectional = function()
    local first = capture_records()
    local second = capture_records()
    local repeat_difference = first_difference(second, first, "old_behavior_observations[review-replayer-merge-ready][repeat]")
    if repeat_difference ~= nil or canonical_json(second) ~= canonical_json(first) then
      error("second OLD direct-constructor runtime capture differs at " .. tostring(repeat_difference or "canonical-json"), 0)
    end
    t.eq(#first, #FIXTURES, "every production source branch has one merge-ready payload observation")
    local expected = committed_records()
    local inventory_difference = first_difference(first, expected, "old_behavior_observations[review-replayer-merge-ready]")
    if inventory_difference ~= nil or canonical_json(first) ~= canonical_json(expected) then
      error(
        "runtime-bound OLD direct-constructor observation differs at "
          .. tostring(inventory_difference or "canonical-json")
          .. "; runtime_records=" .. canonical_json(first),
        0
      )
    end
  end,
}
