local conv_reconcile = require("devloop.convergence.reconcile")
local entity_read_mocks = require("tests.entity_read_mock_helpers")
local h = require("tests.devloop_helpers")
local observation_support = require("tests.old_behavior_observation_support_helpers")
local testing = require("testkit.testing")
local transition_version = require("contract.transition_version")
local reconcile_department = require("departments.reconcile.main")

local t = h.t
local core = h.core
local JSON_NULL = observation_support.JSON_NULL
local canonical_json = observation_support.canonical_json
local copy_value = observation_support.copy_value
local first_difference = observation_support.first_difference
local json_array = observation_support.json_array
local nullable = observation_support.nullable
local INVENTORY_PATH = "migration/restart-lifecycle.inventory.json"
local SITE = {
  path = "packages/github-devloop-pr/departments/reconcile/main.lua",
  symbol = "pipeline_review",
  ordinal = "versioned_transition_status:reviewing->blocked",
}

local REPO = "owner/repo"
local ISSUE_NUMBER = 42

local function review_reconcile_event()
  return {
    queue = "devloop_review_reconcile",
    payload = h.review_reconcile(),
    now_seconds = 1784048400,
  }
end

local function prepare_fixture(comments)
  h.mock_bot_env()
  h.mock_default_issue_claim(REPO, ISSUE_NUMBER)
  entity_read_mocks.mock_pr_view_selector(t, {
    repo = REPO,
    number = 7,
    comments = comments,
    head = "devloop-owner-repo-42-01HY",
    head_sha = "def456",
    state = "OPEN",
    base_branch = "dev",
    labels = {},
  }, entity_read_mocks.pr_origin_selector, 1)
end

local function observe_real_department(event, comments)
  prepare_fixture(comments)
  return observation_support.observe_department({
    dept = "reconcile",
    from_state = "reviewing",
    transition_kind = "versioned_transition_status",
    run = function()
      return testing.run_fake(reconcile_department, event)
    end,
    codex_runs_for_read = json_array(),
    write_mode = "real",
  })
end

local EFFECTS = {
  ["github-proxy.github_pr_comment_request"] = {
    effect_id = "comment:pr:reconcile-reviewing-blocked",
    sink_kind = "comment",
    authority_class = "lifecycle-authoritative",
  },
  ["github-proxy.github_issue_label_request"] = {
    effect_id = "label:issue:reconcile-reviewing-blocked",
    sink_kind = "label",
    authority_class = "lifecycle-authoritative",
  },
}

local function outcome_status(probe, decision, apply)
  t.eq(probe.outcome, "apply", "review reconcile reaches only the apply CAS regime")
  t.eq(decision.outcome, "applied", "review reconcile routes the apply decision")
  t.eq(apply.to_state, "blocked", "review reconcile applies blocked")
  return "apply", "apply", decision.outcome
end

local function effects_from_raises(raises)
  local effects = json_array()
  local writes = json_array()
  for ordinal, raised in ipairs(raises) do
    local shape = EFFECTS[raised.queue]
    if shape == nil then
      error("unclassified OLD reconcile raise: " .. tostring(raised.queue))
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

local function record_lineage(payload, incoming_version, decision)
  return {
    proposal_id = decision.proposal_id,
    review_proposal_id = payload.review_proposal_id,
    issue_version = payload.issue_version,
    round = payload.round,
    head_sha = payload.head_sha,
    dedup_key = payload.dedup_key,
    incoming_version = incoming_version,
    source_ref = copy_value(payload.source_ref),
  }
end

local function build_record(event, result, captured)
  return observation_support.build_record({
    t = t,
    dept = "reconcile",
    event = event,
    result = result,
    captured = captured,
    owner = "github-devloop-pr",
    site = SITE,
    observation_prefix = "writer:github-devloop-pr:reconcile-reviewing-blocked",
    transition_kind = "versioned_transition_status",
    outcome_status = outcome_status,
    effects_from_raises = effects_from_raises,
    lineage = function(payload, probe, decision)
      return record_lineage(payload, probe.incoming_version, decision)
    end,
  })
end

local function capture_apply_record()
  local event = review_reconcile_event()
  local current_version = event.payload.issue_version
  local result, captured = observe_real_department(event, json_array({
    core.state_marker(event.payload.proposal_id, "reviewing", current_version),
  }))
  local record = build_record(event, result, captured)
  local expected_version = conv_reconcile.review_reconcile_terminal_state_version(
    current_version,
    event.payload.round
  )

  t.eq(captured.probes[1].incoming_version, expected_version, "reconcile derives the terminal version from current state")
  t.is_true(
    transition_version.compare(expected_version, current_version) > 0,
    "reconcile terminal version is newer than the live reviewing marker"
  )
  t.eq(record.old_inputs.target_version, JSON_NULL, "four-argument versioned target_version")
  t.eq(record.old_outcome.status, "apply", "captured reconcile status")
  t.eq(record.old_outcome.reason_code, "apply", "captured reconcile disposition")
  t.eq(record.old_outcome.cas_outcome, "applied", "captured reconcile CAS outcome")
  t.eq(#record.old_outcome.emitted_effects, 2, "review reconcile emits comment and label effects")
  t.eq(captured.liveness_read_count, 0, "deterministic reconcile never reads Codex liveness")
  return record
end

local function build_pre_cas_guard_record(event, result, captured, name, expected_outcome)
  local decision = captured.decisions[1]
  local current = decision.current
  local incoming_version = conv_reconcile.review_reconcile_terminal_state_version(
    event.payload.issue_version,
    event.payload.round
  )
  local emitted_effects, observable_writes = effects_from_raises(result.raises)
  return {
    schema = "restart-old-behavior-observation.v2",
    observation_id = table.concat({
      "writer:github-devloop-pr:reconcile-reviewing-blocked",
      "blocked",
      expected_outcome,
      name,
      "none",
    }, "/"),
    owner = "github-devloop-pr",
    site = copy_value(SITE),
    boundary = "writer",
    typed_intent = {
      kind = "versioned_transition_status",
      source_state = "reviewing",
      source_boundary = JSON_NULL,
      target = "blocked",
      cause_schema_id = event.payload.schema,
      generation_epoch = {
        current_version = nullable(current.version),
        incoming_version = incoming_version,
        target_version = JSON_NULL,
      },
      lineage = record_lineage(event.payload, incoming_version, decision),
    },
    old_inputs = {
      current_fact = {
        state = nullable(current.state),
        version = nullable(current.version),
        stage_rank = nullable(current.stage_rank),
      },
      caller_from_states = json_array({ "reviewing" }),
      incoming_version = incoming_version,
      target_version = JSON_NULL,
      handoff_reference = JSON_NULL,
    },
    old_outcome = {
      status = decision.outcome,
      reason_code = name,
      cas_outcome = decision.outcome,
      emitted_effects = emitted_effects,
      observable_writes = observable_writes,
      handoff_direct_lookup_count = captured.handoff_direct_lookup_count,
      timeout_evidence_source = JSON_NULL,
    },
    evidence_refs = json_array({
      {
        kind = "runtime-cas-decision",
        ref = "devloop.logging.log_cas_decision:" .. tostring(decision.outcome),
      },
      {
        kind = "runtime-event-source",
        ref = tostring(event.payload.source_ref and event.payload.source_ref.ref),
      },
    }),
  }
end

local function capture_pre_cas_guard(name, comments, expected_outcome)
  local event = review_reconcile_event()
  local result, captured = observe_real_department(event, comments(event))
  t.eq(#captured.probes, 0, name .. ": guard returns before the writer CAS probe")
  t.eq(#captured.decisions, 1, name .. ": guard logs one decision")
  t.eq(captured.decisions[1].dept, "reconcile", name .. ": disposition belongs to reconcile")
  t.eq(captured.decisions[1].from_state, "reviewing", name .. ": disposition retains writer source")
  t.eq(captured.decisions[1].to_state, "blocked", name .. ": disposition retains writer target")
  t.eq(captured.decisions[1].outcome, expected_outcome, name .. ": captured guard disposition")
  t.eq(#captured.applies, 0, name .. ": guard emits no apply")
  t.eq(#captured.raises, 0, name .. ": guard emits no raise")
  t.eq(#result.raises, 0, name .. ": runtime emits no effect")
  t.eq(captured.liveness_read_count, 0, name .. ": guard never reads Codex liveness")
  local record = build_pre_cas_guard_record(event, result, captured, name, expected_outcome)
  t.eq(record.old_inputs.target_version, JSON_NULL, name .. ": pre-CAS target_version is null")
  t.eq(record.old_outcome.status, expected_outcome, name .. ": status retains exact logged disposition")
  t.eq(record.old_outcome.cas_outcome, expected_outcome, name .. ": CAS outcome retains exact logged disposition")
  t.eq(#record.old_outcome.emitted_effects, 0, name .. ": no emitted effects")
  t.eq(#record.old_outcome.observable_writes, 0, name .. ": no observable writes")
  return record
end

local PRE_CAS_GUARDS = {
  {
    name = "review-reconcile-marker-visible",
    expected_outcome = "skip-idempotent(review reconcile marker already visible)",
    comments = function(event)
      local version = conv_reconcile.review_reconcile_terminal_state_version(
        event.payload.issue_version,
        event.payload.round
      )
      return json_array({
        core.build_review_reconcile_comment_request(
          REPO,
          tostring(ISSUE_NUMBER),
          event.payload,
          "drop",
          "already done",
          version
        ).body,
      })
    end,
  },
  {
    name = "already-terminal",
    expected_outcome = "skip-idempotent(already terminal)",
    comments = function(event)
      local version = conv_reconcile.review_reconcile_terminal_state_version(
        event.payload.issue_version,
        event.payload.round
      )
      return json_array({
        core.state_marker(event.payload.proposal_id, "blocked", version),
      })
    end,
  },
  {
    name = "state-advanced",
    expected_outcome = "skip-stale(state-advanced)",
    comments = function(event)
      return json_array({
        core.state_marker(event.payload.proposal_id, "fixing", event.payload.issue_version .. "/fix/1"),
      })
    end,
  },
}

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
  test_reconcile_old_observations_are_runtime_bound_to_inventory = function()
    local actual = json_array()
    for _, fixture in ipairs(PRE_CAS_GUARDS) do
      local ok, record_or_failure = pcall(
        capture_pre_cas_guard,
        fixture.name,
        fixture.comments,
        fixture.expected_outcome
      )
      if not ok then
        error("OLD reconcile guard " .. fixture.name .. " failed: " .. tostring(record_or_failure), 0)
      end
      table.insert(actual, record_or_failure)
    end

    table.insert(actual, capture_apply_record())
    table.sort(actual, function(left, right)
      return tostring(left.observation_id) < tostring(right.observation_id)
    end)
    local expected = committed_records()
    local difference = first_difference(actual, expected, "old_behavior_observations[reconcile]")
    if difference ~= nil or canonical_json(actual) ~= canonical_json(expected) then
      error(
        "runtime-bound OLD reconcile observation differs at " .. tostring(difference or "canonical-json")
          .. "; runtime_records=" .. canonical_json(actual),
        0
      )
    end
  end,
}
