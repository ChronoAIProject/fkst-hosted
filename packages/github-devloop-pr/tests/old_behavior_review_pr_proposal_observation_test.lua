local devloop_base = require("devloop.base")
local devloop_logging = require("devloop.logging")
local devloop_state = require("devloop.state")
local entity_read_mocks = require("tests.entity_read_mock_helpers")
local h = require("tests.devloop_helpers")
local m_builders = require("devloop.markers.builders")
local observation_support = require("testkit.old_behavior_observation_support")
local payloads_builders = require("devloop.payloads.builders")
local testing = require("testkit.testing")
local review_pr_department = require("departments.review_pr.main")

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
  path = "packages/github-devloop-pr/departments/review_pr/main.lua",
  symbol = "pipeline",
  ordinal = "review proposal payload",
}

local REPO = "owner/repo"
local ISSUE_NUMBER = 42
local PR_NUMBER = 7
local PROPOSAL_ID = "github-devloop/issue/owner/repo/42"
local BRANCH = "devloop-owner-repo-42-01HY"
local BASE_BRANCH = "dev"
local HEAD_SHA = "def456"
local BASE_VERSION = "ready/consensus-github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z"
local PR_SOURCE_REF = { kind = "external", ref = "owner/repo#pr/7" }

local FIXTURES = {
  {
    name = "normal-risk-fallback-worktree",
    version = BASE_VERSION .. "/review-loop/701",
    tick = "2026-06-03T02:03:01Z",
    expected_worktree = ".",
  },
  {
    name = "normal-risk-existing-worktree",
    version = BASE_VERSION .. "/review-loop/704",
    tick = "2026-06-03T02:03:04Z",
    existing_worktree = true,
  },
  {
    name = "high-risk-existing-worktree",
    version = BASE_VERSION .. "/review-loop/702",
    tick = "2026-06-03T02:03:02Z",
    high_risk = true,
    existing_worktree = true,
  },
  {
    name = "high-risk-fallback-worktree",
    version = BASE_VERSION .. "/review-loop/705",
    tick = "2026-06-03T02:03:05Z",
    high_risk = true,
    expected_worktree = ".",
  },
  {
    name = "redrive-delivery-identity",
    version = BASE_VERSION .. "/review-loop/703",
    tick = "2026-06-03T02:03:03Z",
    redrive = true,
    expected_worktree = ".",
  },
  {
    name = "normal-risk-existing-worktree-redrive",
    version = BASE_VERSION .. "/review-loop/706",
    tick = "2026-06-03T02:03:06Z",
    redrive = true,
    existing_worktree = true,
  },
  {
    name = "high-risk-fallback-worktree-redrive",
    version = BASE_VERSION .. "/review-loop/707",
    tick = "2026-06-03T02:03:07Z",
    high_risk = true,
    redrive = true,
    expected_worktree = ".",
  },
  {
    name = "high-risk-existing-worktree-redrive",
    version = BASE_VERSION .. "/review-loop/708",
    tick = "2026-06-03T02:03:08Z",
    high_risk = true,
    redrive = true,
    existing_worktree = true,
  },
}

local function reviewing_event(fixture)
  local payload = payloads_builders.build_devloop_reviewing_payload({
    proposal_id = PROPOSAL_ID,
    impl_version = fixture.version,
  }, PR_NUMBER, copy_value(PR_SOURCE_REF), fixture.version)
  if fixture.redrive then
    local review_id = devloop_base.pr_review_proposal_id(REPO, PR_NUMBER, fixture.version, HEAD_SHA)
    local delivery = devloop_base.pr_review_redrive_delivery_dedup_key(
      review_id,
      "restart-liveness-v2/reviewing/reviewing.active/live_defer_heartbeat-v1/review-converge-round-missing/1783840000000.0",
      2
    )
    payload.dedup_key = delivery
    payload.review_delivery_dedup_key = delivery
  end
  return {
    queue = "devloop_reviewing",
    ts = fixture.tick,
    payload = payload,
  }
end

local function trusted_comment(body, id)
  return {
    id = id,
    body = body,
    author_login = "fkst-test-bot",
    created_at = "2026-06-03T01:00:00Z",
  }
end

local function pr_comments(fixture)
  local comments = json_array({
    trusted_comment(m_builders.pr_origin_marker(
      PROPOSAL_ID,
      tostring(ISSUE_NUMBER),
      BRANCH,
      fixture.version,
      BASE_BRANCH
    ), "IC_origin_" .. fixture.name),
    trusted_comment(core.state_marker(PROPOSAL_ID, "reviewing", fixture.version), "IC_state_" .. fixture.name),
  })
  return comments
end

local function mock_risk_reads_for_two_runs()
  for _ = 1, 2 do
    for _, fixture in ipairs(FIXTURES) do
      if fixture.high_risk then
        h.mock_pr_high_risk_diff_name_only()
      else
        h.mock_pr_normal_risk_diff_name_only()
      end
    end
  end
end

local function prepare_fixture(fixture, event)
  h.mock_bot_env()
  local expected_worktree = "."
  if fixture.existing_worktree then
    expected_worktree = devloop_base.implement_worktree_path(
      "/tmp/fkst-packages-test/github-devloop/runtime",
      REPO,
      ISSUE_NUMBER,
      fixture.version
    )
    t.mock_command(core.path_is_directory_cmd(expected_worktree), {
      stdout = "",
      stderr = "",
      exit_code = 0,
    })
  else
    t.mock_command("/worktrees/devloop-", {
      stdout = "",
      stderr = "",
      exit_code = 1,
    })
  end
  h.mock_context_bundle(event.payload)
  entity_read_mocks.mock_pr_view_selector(t, {
    repo = REPO,
    number = PR_NUMBER,
    comments = pr_comments(fixture),
    head = BRANCH,
    head_sha = HEAD_SHA,
    base_branch = BASE_BRANCH,
    state = "OPEN",
    labels = {},
  }, entity_read_mocks.pr_origin_selector, 1)
  entity_read_mocks.mock_issue_view_selector(t, {
    repo = REPO,
    number = ISSUE_NUMBER,
    title = "Capture review proposal constructor",
    labels = { "fkst-dev:reviewing" },
    comments = json_array({
      trusted_comment(core.state_marker(PROPOSAL_ID, "reviewing", fixture.version), "IC_issue_state_" .. fixture.name),
    }),
    assignees = { "fkst-test-bot" },
    author_login = "fkst-test-bot",
  }, "title,labels,comments,assignees,author", 1)
  fixture.expected_worktree = expected_worktree
end

local function capture_runtime(fixture)
  local event = reviewing_event(fixture)
  prepare_fixture(fixture, event)
  local constructor_calls = json_array()
  local decisions = json_array()
  local original_builder = payloads_builders.build_board_pr_review_proposal
  local original_decision = devloop_logging.log_cas_decision

  payloads_builders.build_board_pr_review_proposal = function(...)
    local payload = original_builder(...)
    table.insert(constructor_calls, { payload = payload })
    return payload
  end
  devloop_logging.log_cas_decision = function(dept, proposal_id, current, from_state, to_state, outcome, reason)
    if dept == "review_pr" and outcome == "applied" then
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

  local ok, result = pcall(testing.run_fake, review_pr_department, event)
  devloop_logging.log_cas_decision = original_decision
  payloads_builders.build_board_pr_review_proposal = original_builder
  if not ok then
    error(result, 0)
  end

  t.eq(#constructor_calls, 1, fixture.name .. ": real dispatch calls the review proposal constructor once")
  t.eq(#decisions, 1, fixture.name .. ": real dispatch reaches the applied review branch")
  local proposal_raises = json_array()
  for _, raised in ipairs(result.raises) do
    if raised.queue == "consensus.proposal" then
      table.insert(proposal_raises, copy_value(raised))
    end
  end
  t.eq(#proposal_raises, 1, fixture.name .. ": real dispatch emits one consensus proposal")
  local constructor_payload = copy_value(constructor_calls[1].payload)
  t.eq(
    canonical_json(proposal_raises[1].payload),
    canonical_json(constructor_payload),
    fixture.name .. ": emitted payload exactly matches the final constructor table"
  )
  t.eq(constructor_payload.worktree, fixture.expected_worktree, fixture.name .. ": production worktree branch")
  if fixture.high_risk then
    t.eq(canonical_json(constructor_payload.angles), canonical_json(json_array({
      "teleology", "parsimony", "fidelity", "high-risk",
    })), fixture.name .. ": high-risk branch adds the production angle set")
  else
    t.eq(constructor_payload.angles, nil, fixture.name .. ": normal-risk branch omits custom angles")
  end
  if fixture.redrive then
    t.eq(constructor_payload.dedup_key, event.payload.review_delivery_dedup_key, "redrive identity reaches emitted proposal")
  end
  return event, constructor_payload, decisions[1], proposal_raises[1]
end

local function build_record(fixture)
  local event, payload, decision, proposal_raise = capture_runtime(fixture)
  return {
    schema = "restart-old-behavior-observation.v2",
    observation_id = "ctor:github-devloop-pr:review-pr-proposal/" .. fixture.name,
    owner = "github-devloop-pr",
    site = copy_value(SITE),
    boundary = "writer",
    typed_intent = {
      kind = "direct_constructor",
      source_state = "reviewing",
      source_boundary = event.queue,
      target = "consensus.proposal",
      cause_schema_id = event.payload.schema,
      generation_epoch = {
        current_version = decision.current.version,
        source_version = event.payload.version,
        payload_version = payload.dedup_key,
        review_delivery_dedup_key = nullable(event.payload.review_delivery_dedup_key),
      },
      lineage = {
        proposal_id = event.payload.proposal_id,
        review_proposal_id = payload.proposal_id,
        pr_number = event.payload.pr_number,
        reviewed_head_sha = HEAD_SHA,
        source_ref = copy_value(payload.source_ref),
        angles = nullable(copy_value(payload.angles)),
        worktree = payload.worktree,
      },
    },
    old_inputs = {
      current_fact = {
        state = decision.current.state,
        version = decision.current.version,
        stage_rank = decision.current.stage_rank,
      },
      caller_from_states = json_array({ "reviewing" }),
      incoming_version = event.payload.version,
      target_version = payload.dedup_key,
      handoff_reference = nullable(copy_value(event.payload.reviewing_hand_off)),
    },
    old_outcome = {
      status = "constructed",
      reason_code = fixture.name,
      cas_outcome = "not-applicable-direct-constructor",
      emitted_effects = json_array({
        {
          effect_id = "queue:consensus.proposal",
          sink_kind = "queue",
          authority_class = "lifecycle-authoritative",
          ordinal = 1,
        },
      }),
      observable_writes = json_array({
        {
          effect_id = "queue:consensus.proposal",
          queue = proposal_raise.queue,
          payload = copy_value(proposal_raise.payload),
        },
      }),
      handoff_direct_lookup_count = 0,
      timeout_evidence_source = JSON_NULL,
    },
    evidence_refs = json_array({
      {
        kind = "runtime-constructor-call",
        ref = "devloop.payloads.builders.build_board_pr_review_proposal",
      },
      {
        kind = "runtime-event-source",
        ref = event.payload.source_ref.ref,
      },
      {
        kind = "production-callsite",
        ref = "packages/github-devloop-pr/departments/review_pr/main.lua:166",
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
  test_review_pr_constructor_canonical_json_rejects_empty_array_object_drift = function()
    t.is_true(JSON_ARRAY_TAG ~= JSON_OBJECT_TAG, "json.decode preserves array versus object shape")
    local expected = { emitted_effects = json_array() }
    local drifted = copy_value(expected)
    drifted.emitted_effects = {}
    local root = "old_behavior_observations[review-pr-proposal][negative_control]"
    local difference = first_difference(drifted, expected, root)
    t.eq(canonical_json(expected.emitted_effects), "[]", "empty effects retain array shape")
    t.is_true(canonical_json(drifted) ~= canonical_json(expected), "empty array to object drift changes JSON")
    t.is_true(difference ~= nil and difference:find(root .. ".emitted_effects", 1, true) ~= nil)
  end,

  test_review_pr_proposal_old_observations_are_real_dispatch_runtime_bound_and_bidirectional = function()
    mock_risk_reads_for_two_runs()
    local first = capture_records()
    local second = capture_records()
    local repeat_difference = first_difference(second, first, "old_behavior_observations[review-pr-proposal][repeat]")
    if repeat_difference ~= nil or canonical_json(second) ~= canonical_json(first) then
      error("second OLD direct-constructor runtime capture differs at " .. tostring(repeat_difference or "canonical-json"), 0)
    end
    t.eq(#first, #FIXTURES, "every production payload branch has one observation")
    local expected = committed_records()
    local inventory_difference = first_difference(first, expected, "old_behavior_observations[review-pr-proposal]")
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
