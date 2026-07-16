local devloop_logging = require("devloop.logging")
local entity_read_mocks = require("tests.entity_read_mock_helpers")
local execution_start = require("devloop.execution_start")
local h = require("tests.devloop_helpers")
local observation_support = require("testkit.old_behavior_observation_support")
local testing = require("testkit.testing")
local execute_start_department = require("departments.execute_start.main")

local t = h.t
local JSON_NULL = observation_support.JSON_NULL
local JSON_ARRAY_TAG = observation_support.JSON_ARRAY_TAG
local JSON_OBJECT_TAG = observation_support.JSON_OBJECT_TAG
local canonical_json = observation_support.canonical_json
local copy_value = observation_support.copy_value
local first_difference = observation_support.first_difference
local json_array = observation_support.json_array
local INVENTORY_PATH = "migration/restart-lifecycle.inventory.json"
local SITE = {
  path = "packages/github-devloop/departments/execute_start/main.lua",
  symbol = "pipeline",
  ordinal = "build_consensus_proposal",
}

local PROPOSAL_ID = "github-devloop/issue/owner/repo/42"
local REQUEST_VERSION = "intake/github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z"
local SOURCE_REF = { kind = "external", ref = "owner/repo#issue/42" }

local function execution_request()
  return execution_start.build_execution_request_payload({
    proposal_id = PROPOSAL_ID,
    dedup_key = REQUEST_VERSION,
    source_ref = copy_value(SOURCE_REF),
    origin = {
      package = "github-devloop-intake-default",
      route = "default",
      decision = "enable",
    },
    service_class = "expedite",
  })
end

local function event_for(request)
  return {
    queue = "devloop_execute_request",
    ts = "2026-06-03T01:02:04Z",
    payload = request,
  }
end

local function prepare_fixture(request)
  h.mock_bot_env()
  entity_read_mocks.mock_issue_view_selector(t, {
    repo = "owner/repo",
    number = 42,
    title = "Capture execute-start consensus proposal",
    body = "Record the production constructor payload.",
    updated_at = "2026-06-03T01:02:03Z",
    state = "OPEN",
    labels = {},
    comments = {},
    assignees = { "fkst-test-bot" },
    author_login = "fkst-test-bot",
  }, "title,body,createdAt,updatedAt,labels,comments,state,assignees,author", 1)
  h.mock_context_bundle(request)
end

local function capture_runtime()
  local request = execution_request()
  local event = event_for(request)
  prepare_fixture(request)
  local constructor_calls = json_array()
  local decisions = json_array()
  local original_builder = execution_start.build_execution_start_effects
  local original_decision = devloop_logging.log_cas_decision

  execution_start.build_execution_start_effects = function(core, repo, issue_number, source, current, event_ts, dept)
    local effects = original_builder(core, repo, issue_number, source, current, event_ts, dept)
    table.insert(constructor_calls, {
      request = copy_value(source),
      current = copy_value(current),
      payload = effects and effects.proposal,
    })
    return effects
  end
  devloop_logging.log_cas_decision = function(dept, proposal_id, current, from_state, to_state, outcome, reason)
    if dept == "execute_start" then
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

  local ok, result = pcall(testing.run_fake, execute_start_department, event)
  devloop_logging.log_cas_decision = original_decision
  execution_start.build_execution_start_effects = original_builder
  if not ok then
    error(result, 0)
  end

  t.eq(#constructor_calls, 1, "real execute_start dispatch calls the constructor exactly once")
  local proposal_raises = json_array()
  for _, raised in ipairs(result.raises) do
    if raised.queue == "consensus.proposal" then
      table.insert(proposal_raises, copy_value(raised))
    end
  end
  t.eq(#proposal_raises, 1, "real execute_start dispatch emits one consensus proposal")
  local constructor = constructor_calls[1]
  constructor.payload = copy_value(constructor.payload)
  t.eq(
    canonical_json(proposal_raises[1].payload),
    canonical_json(constructor.payload),
    "emitted proposal exactly matches the direct constructor result"
  )
  return event, constructor, decisions, proposal_raises[1]
end

local function build_record()
  local event, constructor, decisions, proposal_raise = capture_runtime()
  local payload = constructor.payload
  t.eq(#decisions, 0, "successful execute_start proposal construction has no competing CAS disposition")
  return {
    schema = "restart-old-behavior-observation.v2",
    observation_id = "ctor:github-devloop:execute-start-proposal/intake-enable",
    owner = "github-devloop",
    site = copy_value(SITE),
    boundary = "writer",
    typed_intent = {
      kind = "direct_constructor",
      source_state = "execution-request",
      source_boundary = event.queue,
      target = "consensus.proposal",
      cause_schema_id = event.payload.schema,
      generation_epoch = {
        current_version = JSON_NULL,
        source_version = event.payload.dedup_key,
        payload_version = payload.dedup_key,
      },
      lineage = {
        proposal_id = payload.proposal_id,
        issue_number = 42,
        source_ref = copy_value(payload.source_ref),
        intake_hand_off = copy_value(payload.intake_hand_off),
      },
    },
    old_inputs = {
      current_fact = {
        state = JSON_NULL,
        version = JSON_NULL,
        stage_rank = JSON_NULL,
      },
      caller_from_states = json_array({ "execution-request" }),
      incoming_version = event.payload.dedup_key,
      target_version = payload.dedup_key,
      handoff_reference = copy_value(payload.intake_hand_off),
    },
    old_outcome = {
      status = "constructed",
      reason_code = "intake-enable-proposal",
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
        ref = "devloop.execution_start.build_execution_start_effects",
      },
      {
        kind = "runtime-event-source",
        ref = event.payload.source_ref.ref,
      },
      {
        kind = "production-callsite",
        ref = "packages/github-devloop/departments/execute_start/main.lua:51",
      },
    }),
  }
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
  test_execute_start_constructor_canonical_json_rejects_empty_array_object_drift = function()
    t.is_true(JSON_ARRAY_TAG ~= JSON_OBJECT_TAG, "json.decode preserves array versus object shape")
    local expected = { emitted_effects = json_array() }
    local drifted = copy_value(expected)
    drifted.emitted_effects = {}
    local root = "old_behavior_observations[execute-start-proposal][negative_control]"
    local difference = first_difference(drifted, expected, root)
    t.eq(canonical_json(expected.emitted_effects), "[]", "empty effects retain array shape")
    t.is_true(canonical_json(drifted) ~= canonical_json(expected), "empty array to object drift changes JSON")
    t.is_true(difference ~= nil and difference:find(root .. ".emitted_effects", 1, true) ~= nil)
  end,

  test_execute_start_proposal_old_observation_is_real_dispatch_runtime_bound_and_bidirectional = function()
    local first = json_array({ build_record() })
    local second = json_array({ build_record() })
    local repeat_difference = first_difference(second, first, "old_behavior_observations[execute-start-proposal][repeat]")
    if repeat_difference ~= nil or canonical_json(second) ~= canonical_json(first) then
      error("second OLD direct-constructor runtime capture differs at " .. tostring(repeat_difference or "canonical-json"), 0)
    end
    local expected = committed_records()
    local inventory_difference = first_difference(first, expected, "old_behavior_observations[execute-start-proposal]")
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
