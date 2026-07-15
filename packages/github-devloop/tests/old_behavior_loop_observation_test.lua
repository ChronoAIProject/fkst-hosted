local config = require("devloop.config")
local convergence_shared = require("devloop.convergence.shared")
local conv_rounds = require("devloop.convergence.rounds")
local devloop_logging = require("devloop.logging")
local devloop_state = require("devloop.state")
local h = require("tests.devloop_helpers")
local observation_support = require("testkit.old_behavior_observation_support")
local testing = require("testkit.testing")
local loop_department = require("departments.loop.main")

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
  path = "packages/github-devloop/departments/loop/main.lua",
  symbol = "pipeline",
  ordinal = "transition_status:thinking->blocked",
}

local BASE_VERSION = "consensus:github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z"
local QUESTION = "Which source-verifiable fact resolves the remaining gap?"
local ANGLES = {
  { angle = "minimal", verdict = "abstain", digest = "same-evidence-gap" },
}

local EFFECTS = {
  ["consensus.proposal"] = {
    effect_id = "queue:consensus.proposal",
    sink_kind = "queue",
    authority_class = "lifecycle-authoritative",
  },
  ["github-proxy.github_issue_comment_request"] = {
    effect_id = "comment:issue:converge-round",
    sink_kind = "comment",
    authority_class = "lifecycle-authoritative",
  },
}

local function loop_event(extra)
  local payload = h.unresolved({
    dedup_key = BASE_VERSION,
    round = 0,
    narrowed_question = QUESTION,
    angle_digests = ANGLES,
  })
  for key, value in pairs(extra or {}) do
    payload[key] = value
  end
  return {
    queue = "consensus.consensus_converge",
    payload = payload,
    now_seconds = 1784048400,
  }
end

local function state_marker(event, state, version)
  return core.state_marker(event.payload.proposal_id, state, version or BASE_VERSION)
end

local function round_marker(event, round, extra)
  extra = extra or {}
  return conv_rounds.converge_round_marker(
    event.payload.proposal_id,
    extra.version or BASE_VERSION,
    convergence_shared.source_ref_digest(event.payload.source_ref),
    round,
    extra.dedup_key or (round == 0 and BASE_VERSION or BASE_VERSION .. "/loop/" .. tostring(round)),
    extra.narrowed_question or QUESTION,
    extra.angle_digests or ANGLES,
    extra.findings_record,
    extra.essence_stall == true
  )
end

local function prepare_fixture(event, fixture)
  h.mock_bot_env()
  if fixture.needs_context then
    h.mock_context_bundle(event.payload)
  end
  h.mock_issue_loop(
    fixture.labels or { "fkst-dev:thinking" },
    fixture.comments(event),
    fixture.issue_fields
  )
end

local function observe_real_department(event, fixture)
  prepare_fixture(event, fixture)
  return observation_support.observe_department({
    config = config,
    devloop_logging = devloop_logging,
    devloop_state = devloop_state,
    dept = "loop",
    from_state = "thinking",
    transition_kind = "transition_status",
    run = function()
      return testing.run_fake(loop_department, event)
    end,
    codex_runs_for_read = json_array(),
    write_mode = "real",
  })
end

local function effects_from_raises(raises)
  local effects = json_array()
  local writes = json_array()
  for ordinal, raised in ipairs(raises) do
    local shape = EFFECTS[raised.queue]
    if shape == nil then
      error("unclassified OLD loop raise: " .. tostring(raised.queue))
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

local function record_lineage(payload, probe)
  return {
    proposal_id = payload.proposal_id,
    base_version = conv_rounds.converge_base_version(payload.dedup_key),
    round = nullable(payload.round),
    dedup_key = payload.dedup_key,
    source_ref = copy_value(payload.source_ref),
    cas_incoming_version = nullable(probe and probe.incoming_version),
    cas_target_version = nullable(probe and probe.target_version),
  }
end

local function build_cas_record(event, result, captured, fixture)
  local record = observation_support.build_record({
    t = t,
    dept = "loop",
    event = event,
    result = result,
    captured = captured,
    owner = "github-devloop",
    site = SITE,
    observation_prefix = "writer:github-devloop:loop-thinking-blocked",
    observation_variant = fixture.name,
    transition_kind = "transition_status",
    outcome_status = function(probe, decision, apply)
      t.eq(probe.outcome, fixture.probe_outcome, fixture.name .. ": plain CAS outcome")
      t.eq(decision.outcome, fixture.decision_outcome, fixture.name .. ": exact logged disposition")
      if fixture.logs_apply then
        t.is_true(type(apply) == "table", fixture.name .. ": effectful branch logs its effect plan")
        t.eq(apply.to_state, nil, fixture.name .. ": loop effect plan never writes blocked")
      else
        t.eq(apply, nil, fixture.name .. ": returning guard logs no apply")
      end
      return fixture.status, fixture.reason_code, decision.outcome
    end,
    effects_from_raises = effects_from_raises,
    lineage = function(payload, probe)
      return record_lineage(payload, probe)
    end,
  })

  local probe = captured.probes[1]
  t.eq(#probe.from_states, 1, fixture.name .. ": one plain CAS source state")
  t.eq(probe.from_states[1], "thinking", fixture.name .. ": plain CAS source")
  t.eq(probe.to_state, "blocked", fixture.name .. ": plain CAS target")
  t.eq(probe.incoming_version, nil, fixture.name .. ": three-argument CAS has no incoming version")
  t.eq(probe.target_version, nil, fixture.name .. ": three-argument CAS has no target version")
  t.eq(captured.liveness_read_count, 0, fixture.name .. ": loop never reads Codex liveness")

  -- The observation schema requires an event lineage version even though the
  -- production transition_status call itself has only three arguments.
  record.old_inputs.incoming_version = event.payload.dedup_key
  record.old_inputs.target_version = JSON_NULL
  record.typed_intent.generation_epoch.incoming_version = JSON_NULL
  record.typed_intent.generation_epoch.target_version = JSON_NULL
  return record
end

local function build_author_guard_record(event, result, captured, fixture)
  t.eq(#captured.probes, 0, fixture.name .. ": author guard returns before plain CAS")
  t.eq(#captured.decisions, 1, fixture.name .. ": author guard logs one decision")
  t.eq(#captured.applies, 0, fixture.name .. ": author guard logs no apply")
  t.eq(#captured.raises, 0, fixture.name .. ": author guard logs no raise")
  t.eq(#result.raises, 0, fixture.name .. ": author guard emits no effect")
  t.eq(captured.liveness_read_count, 0, fixture.name .. ": author guard never reads Codex liveness")
  local decision = captured.decisions[1]
  local emitted_effects, observable_writes = effects_from_raises(result.raises)
  return {
    schema = "restart-old-behavior-observation.v2",
    observation_id = table.concat({
      "writer:github-devloop:loop-thinking-blocked",
      fixture.name,
      "blocked",
      "guard-return",
      fixture.reason_code,
      "none",
    }, "/"),
    owner = "github-devloop",
    site = copy_value(SITE),
    boundary = "writer",
    typed_intent = {
      kind = "transition_status",
      source_state = "thinking",
      source_boundary = JSON_NULL,
      target = "blocked",
      cause_schema_id = event.payload.schema,
      generation_epoch = {
        current_version = nullable(decision.current.version),
        incoming_version = JSON_NULL,
        target_version = JSON_NULL,
      },
      lineage = record_lineage(event.payload),
    },
    old_inputs = {
      current_fact = {
        state = nullable(decision.current.state),
        version = nullable(decision.current.version),
        stage_rank = nullable(decision.current.stage_rank),
      },
      caller_from_states = json_array({ "thinking" }),
      incoming_version = event.payload.dedup_key,
      target_version = JSON_NULL,
      handoff_reference = JSON_NULL,
    },
    old_outcome = {
      status = "guard-return",
      reason_code = fixture.reason_code,
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

local FIXTURES = {
  {
    name = "author-not-authorized",
    reason_code = "non-whitelisted-author",
    event = function() return loop_event() end,
    comments = function(event) return json_array({ state_marker(event, "thinking") }) end,
    issue_fields = { author_login = "ordinary-user" },
    pre_cas = true,
  },
  {
    name = "already-at-target",
    probe_outcome = "idempotent",
    decision_outcome = "skip-idempotent(already at to_state)",
    status = "idempotent",
    reason_code = "already-at-target",
    event = function() return loop_event() end,
    labels = { "fkst-dev:blocked" },
    comments = function(event) return json_array({ state_marker(event, "blocked") }) end,
  },
  {
    name = "state-advanced",
    probe_outcome = "stale",
    decision_outcome = "skip-advanced-or-diverged",
    status = "stale",
    reason_code = "state-advanced",
    event = function() return loop_event() end,
    labels = { "fkst-dev:ready" },
    comments = function(event) return json_array({ state_marker(event, "ready", BASE_VERSION .. "/ready/1") }) end,
  },
  {
    name = "lineage-terminal-external-evidence",
    probe_outcome = "apply",
    decision_outcome = "applied",
    status = "apply",
    reason_code = "lineage-terminal-external-evidence",
    logs_apply = true,
    event = function()
      return loop_event({ dedup_key = BASE_VERSION .. "/loop/1", round = 1 })
    end,
    comments = function(event)
      return json_array({
        state_marker(event, "thinking"),
        round_marker(event, 0, { findings_record = "open:\nexternal evidence remains", essence_stall = true }),
      })
    end,
  },
  {
    name = "lineage-terminal-continuation-budget",
    probe_outcome = "apply",
    decision_outcome = "applied",
    status = "apply",
    reason_code = "lineage-terminal-continuation-budget",
    logs_apply = true,
    event = function()
      return loop_event({ dedup_key = BASE_VERSION .. "/loop/2", round = 2 })
    end,
    comments = function(event)
      return json_array({
        state_marker(event, "thinking"),
        round_marker(event, 1, { findings_record = "open:\nsecond resolvable finding" }),
      })
    end,
  },
  {
    name = "lineage-terminal-no-semantic-progress",
    probe_outcome = "apply",
    decision_outcome = "applied",
    status = "apply",
    reason_code = "lineage-terminal-no-semantic-progress",
    logs_apply = true,
    event = function()
      return loop_event({ dedup_key = BASE_VERSION .. "/loop/4", round = 4 })
    end,
    comments = function(event)
      return json_array({
        state_marker(event, "thinking"),
        round_marker(event, 1, { findings_record = "open:\nfirst unchanged finding" }),
        round_marker(event, 2, { findings_record = "open:\nsecond unchanged finding" }),
        round_marker(event, 3, { findings_record = "open:\nthird unchanged finding" }),
      })
    end,
  },
  {
    name = "round-lineage-already-advanced",
    probe_outcome = "apply",
    decision_outcome = "skip-stale(converge round lineage already advanced)",
    status = "apply",
    reason_code = "round-lineage-already-advanced",
    event = function() return loop_event({ round = 0 }) end,
    comments = function(event)
      return json_array({ state_marker(event, "thinking"), round_marker(event, 0) })
    end,
  },
  {
    name = "round-gap",
    probe_outcome = "apply",
    decision_outcome = "skip-stale(converge round gap)",
    status = "apply",
    reason_code = "round-gap",
    event = function()
      return loop_event({ dedup_key = BASE_VERSION .. "/loop/2", round = 2 })
    end,
    comments = function(event)
      return json_array({ state_marker(event, "thinking"), round_marker(event, 0) })
    end,
  },
  {
    name = "current-terminal-external-evidence",
    probe_outcome = "apply",
    decision_outcome = "applied",
    status = "apply",
    reason_code = "current-terminal-external-evidence",
    logs_apply = true,
    event = function()
      return loop_event({
        findings_record = "open:\nno source-verifiable evidence remains",
        essence_stall = true,
      })
    end,
    comments = function(event) return json_array({ state_marker(event, "thinking") }) end,
  },
  {
    name = "current-terminal-continuation-budget",
    probe_outcome = "apply",
    decision_outcome = "applied",
    status = "apply",
    reason_code = "current-terminal-continuation-budget",
    logs_apply = true,
    event = function()
      return loop_event({
        dedup_key = BASE_VERSION .. "/loop/1",
        round = 1,
        findings_record = "open:\nsecond resolvable finding",
      })
    end,
    comments = function(event)
      return json_array({
        state_marker(event, "thinking"),
        round_marker(event, 0, { findings_record = "open:\nfirst resolvable finding" }),
      })
    end,
  },
  {
    name = "continue-true-stall-not-met",
    probe_outcome = "apply",
    decision_outcome = "applied",
    status = "apply",
    reason_code = "continue-true-stall-not-met",
    logs_apply = true,
    event = function() return loop_event() end,
    comments = function(event) return json_array({ state_marker(event, "thinking") }) end,
    needs_context = true,
  },
}

local function capture_records()
  local records = json_array()
  for _, fixture in ipairs(FIXTURES) do
    local event = fixture.event()
    local result, captured = observe_real_department(event, fixture)
    local record = fixture.pre_cas
      and build_author_guard_record(event, result, captured, fixture)
      or build_cas_record(event, result, captured, fixture)
    table.insert(records, record)
  end
  table.sort(records, function(left, right)
    return tostring(left.observation_id) < tostring(right.observation_id)
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
  test_loop_old_observations_are_runtime_bound_and_deterministic = function()
    local first = capture_records()
    local second = capture_records()
    local repeat_difference = first_difference(second, first, "old_behavior_observations[loop-repeat]")
    if repeat_difference ~= nil or canonical_json(second) ~= canonical_json(first) then
      error("second OLD loop runtime capture differs at " .. tostring(repeat_difference or "canonical-json"), 0)
    end

    local expected = committed_records()
    local inventory_difference = first_difference(first, expected, "old_behavior_observations[loop]")
    if inventory_difference ~= nil or canonical_json(first) ~= canonical_json(expected) then
      error(
        "runtime-bound OLD loop observation differs at " .. tostring(inventory_difference or "canonical-json")
          .. "; runtime_records=" .. canonical_json(first),
        0
      )
    end
  end,
}
