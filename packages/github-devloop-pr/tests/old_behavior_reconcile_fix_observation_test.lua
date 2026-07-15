local conv_reconcile = require("devloop.convergence.reconcile")
local devloop_logging = require("devloop.logging")
local entity_read_mocks = require("tests.entity_read_mock_helpers")
local ci_verdict = require("core.ci_verdict")
local fix_rounds = require("core.fix_rounds")
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
  symbol = "pipeline_fix",
  ordinal = "versioned_transition_status:from_states->blocked",
}

local REPO = "owner/repo"
local ISSUE_NUMBER = 42
local HEAD_SHA = "def456"
local FROM_LABEL = "reviewing|fixing|merge-ready|merging"
local SOURCE_STATES = { "reviewing", "fixing", "merge-ready", "merging" }
local BOUNDED_FROM_LABEL = "fixing|merge-ready|merging"
local BOUNDED_SOURCE_STATES = { "fixing", "merge-ready", "merging" }

local function terminal_event(schema, decide)
  local review_reject = h.fix_reconcile()
  local state = {
    state = "merge-ready",
    version = review_reject.issue_version,
  }
  local context = {
    dept = "merge",
    from_state = "merge-ready",
    proposal_id = review_reject.proposal_id,
    review_proposal_id = review_reject.review_proposal_id,
    review_dedup_key = review_reject.review_dedup_key,
    pr_number = review_reject.pr_number,
    source_ref = review_reject.source_ref,
    bound_head_sha = review_reject.head_sha,
    reason = "fix round terminal fixture",
  }
  local emitted = json_array()
  local original_log_raise = devloop_logging.log_raise
  devloop_logging.log_raise = function(_, _, queue, payload)
    table.insert(emitted, { queue = queue, payload = payload })
  end
  local ok, terminal = pcall(function()
    local decision = decide(state, context)
    while decision.kind == "admit" do
      state.version = decision.version
      decision = decide(state, context)
    end
    return decision
  end)
  devloop_logging.log_raise = original_log_raise
  if not ok then
    error(terminal, 0)
  end
  t.eq(terminal.kind, "terminate", "terminal fixture is produced by the real cap path")
  t.eq(terminal.reconcile.schema, schema, "terminal fixture schema")
  t.eq(#emitted, 2, "terminal fixture emits reconcile and decompose")
  t.eq(emitted[1].queue, "devloop_fix_reconcile", "terminal fixture routes to reconcile")
  t.eq(emitted[1].payload, terminal.reconcile, "terminal event is the producer payload")
  return {
    queue = "devloop_fix_reconcile",
    payload = emitted[1].payload,
    now_seconds = 1784048400,
  }
end

local function fix_reconcile_event()
  return {
    queue = "devloop_fix_reconcile",
    payload = h.fix_reconcile(),
    now_seconds = 1784048400,
  }
end

local function bounded_fix_reconcile_event()
  return terminal_event(fix_rounds.MERGE_GATE_SCHEMA, function(state, context)
    return fix_rounds.admit_or_terminate(state, context)
  end)
end

local function own_ci_reconcile_event()
  return terminal_event(fix_rounds.OWN_CI_SCHEMA, function(state, context)
    local current_pr = {
      head_sha = context.bound_head_sha,
      state = "OPEN",
    }
    return fix_rounds.admit_own_ci_continuation(state, {
      kind = ci_verdict.OWN_CI_RED,
      reason = "own-ci-red",
      ci_failure_key = "head:" .. context.bound_head_sha .. "/checks:test",
      head_sha = context.bound_head_sha,
      current_pr = current_pr,
    }, context)
  end)
end

local function own_ci_policy_invalid_event()
  return terminal_event(fix_rounds.OWN_CI_SCHEMA, function(state, context)
    return fix_rounds.terminate_own_ci_policy_invalid(state, context)
  end)
end

local function check_rollup(conclusion, head_sha)
  return '[{"__typename":"CheckRun","completedAt":"2026-06-03T02:04:04Z","conclusion":"'
    .. tostring(conclusion)
    .. '","detailsUrl":"https://example.invalid/checks/test","name":"test","startedAt":"2026-06-03T02:03:04Z","status":"COMPLETED","workflowName":"test","headSha":"'
    .. tostring(head_sha)
    .. '"}]'
end

local function prepare_fixture(comments, head_sha, surface)
  h.mock_bot_env()
  h.mock_default_issue_claim(REPO, ISSUE_NUMBER)
  if type(surface) == "table" and surface.kind == "own-ci" then
    local observed_head = head_sha or HEAD_SHA
    local conclusion = surface.conclusion or "FAILURE"
    entity_read_mocks.mock_pr_merge_view(t, {
      repo = REPO,
      number = 7,
      comments = comments,
      head = "devloop-owner-repo-42-01HY",
      head_sha = observed_head,
      state = surface.state or "OPEN",
      base_branch = "dev",
      base_sha = "abc123",
      labels = {},
      mergeable = "MERGEABLE",
      merge_state = "UNSTABLE",
      status_check_rollup_json = check_rollup(conclusion, observed_head),
    }, 1)
    if conclusion ~= "SUCCESS" and observed_head == HEAD_SHA then
      h.mock_required_check_runs_for(observed_head, "failure", REPO)
    end
    return
  end
  entity_read_mocks.mock_pr_view_selector(t, {
    repo = REPO,
    number = 7,
    comments = comments,
    head = "devloop-owner-repo-42-01HY",
    head_sha = head_sha or HEAD_SHA,
    state = "OPEN",
    base_branch = "dev",
    labels = {},
  }, entity_read_mocks.pr_origin_selector, 1)
end

local function observe_real_department(event, comments, observed_from, head_sha, surface)
  prepare_fixture(comments, head_sha, surface)
  return observation_support.observe_department({
    dept = "reconcile",
    from_state = observed_from,
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
    effect_id = "comment:pr:reconcile-generic-blocked",
    sink_kind = "comment",
    authority_class = "lifecycle-authoritative",
  },
  ["github-proxy.github_issue_label_request"] = {
    effect_id = "label:issue:reconcile-generic-blocked",
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
      error("unclassified OLD fix reconcile raise: " .. tostring(raised.queue))
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
  local lineage = {
    proposal_id = decision.proposal_id,
    review_proposal_id = payload.review_proposal_id,
    review_dedup_key = payload.review_dedup_key,
    issue_version = payload.issue_version,
    round = payload.round,
    head_sha = payload.head_sha,
    dedup_key = payload.dedup_key,
    incoming_version = incoming_version,
    source_ref = copy_value(payload.source_ref),
  }
  if payload.bound_head_sha ~= nil then
    lineage.bound_head_sha = payload.bound_head_sha
  end
  return lineage
end

local function outcome_status(probe, decision, apply)
  t.eq(probe.outcome, "apply", "fix reconcile source reaches the apply CAS regime")
  t.eq(decision.outcome, "applied", "fix reconcile routes the apply decision")
  t.eq(apply.to_state, "blocked", "fix reconcile applies blocked")
  return "apply", "apply", decision.outcome
end

local function build_apply_record(source_state, event, result, captured, observation_variant)
  return observation_support.build_record({
    t = t,
    dept = "reconcile",
    event = event,
    result = result,
    captured = captured,
    owner = "github-devloop-pr",
    site = SITE,
    observation_prefix = "writer:github-devloop-pr:reconcile-generic-blocked",
    observation_variant = observation_variant or source_state .. "-apply",
    transition_kind = "versioned_transition_status",
    source_state = function(probe)
      return probe.current.state
    end,
    outcome_status = outcome_status,
    effects_from_raises = effects_from_raises,
    lineage = function(payload, probe, decision)
      return record_lineage(payload, probe.incoming_version, decision)
    end,
  })
end

local function capture_apply_record(source_state, options)
  options = options or {}
  local event = (options.event_factory or fix_reconcile_event)()
  local source_states = options.source_states or SOURCE_STATES
  local current_version = event.payload.issue_version
  local result, captured = observe_real_department(event, json_array({
    core.state_marker(event.payload.proposal_id, source_state, current_version),
  }), source_state, nil, options.surface)
  local record = build_apply_record(
    source_state,
    event,
    result,
    captured,
    options.observation_prefix and options.observation_prefix .. source_state .. "-apply" or nil
  )
  local expected_version = conv_reconcile.fix_reconcile_state_version(event.payload.issue_version)

  t.eq(captured.probes[1].incoming_version, expected_version, source_state .. ": helper-derived incoming version")
  t.eq(expected_version, event.payload.issue_version, source_state .. ": fix reconcile preserves issue version")
  t.eq(canonical_json(captured.probes[1].from_states), canonical_json(json_array(source_states)), source_state .. ": multi-source CAS shape")
  t.eq(record.typed_intent.source_state, source_state, source_state .. ": runtime source state")
  t.eq(record.old_inputs.target_version, JSON_NULL, source_state .. ": four-argument target_version")
  t.eq(record.old_outcome.status, "apply", source_state .. ": captured status")
  t.eq(record.old_outcome.cas_outcome, "applied", source_state .. ": captured CAS outcome")
  t.eq(#record.old_outcome.emitted_effects, 2, source_state .. ": comment and label effects")
  t.eq(captured.liveness_read_count, 0, source_state .. ": deterministic reconcile has no Codex liveness read")
  return record
end

local function from_states_from_decision(decision)
  local states = json_array()
  for state in tostring(decision.from_state):gmatch("[^|]+") do
    table.insert(states, state)
  end
  return states
end

local function build_guard_record(fixture, event, result, captured, observation_variant)
  local decision = captured.decisions[1]
  local current = decision.current
  local probe = captured.probes[1]
  local incoming_version = probe and probe.incoming_version
    or conv_reconcile.fix_reconcile_state_version(event.payload.issue_version)
  local target_version = probe and nullable(probe.target_version) or JSON_NULL
  local caller_from_states = probe and copy_value(probe.from_states) or from_states_from_decision(decision)
  local emitted_effects, observable_writes = effects_from_raises(result.raises)
  return {
    schema = "restart-old-behavior-observation.v2",
    observation_id = table.concat({
      "writer:github-devloop-pr:reconcile-generic-blocked",
      observation_variant or fixture.name,
      "blocked",
      fixture.status,
      fixture.reason_code,
      "none",
    }, "/"),
    owner = "github-devloop-pr",
    site = copy_value(SITE),
    boundary = "writer",
    typed_intent = {
      kind = "versioned_transition_status",
      source_state = nullable(current.state),
      source_boundary = JSON_NULL,
      target = "blocked",
      cause_schema_id = event.payload.schema,
      generation_epoch = {
        current_version = nullable(current.version),
        incoming_version = incoming_version,
        target_version = target_version,
      },
      lineage = record_lineage(event.payload, incoming_version, decision),
    },
    old_inputs = {
      current_fact = {
        state = nullable(current.state),
        version = nullable(current.version),
        stage_rank = nullable(current.stage_rank),
      },
      caller_from_states = caller_from_states,
      incoming_version = incoming_version,
      target_version = target_version,
      handoff_reference = JSON_NULL,
    },
    old_outcome = {
      status = fixture.status,
      reason_code = fixture.reason_code,
      cas_outcome = decision.outcome,
      emitted_effects = emitted_effects,
      observable_writes = observable_writes,
      handoff_direct_lookup_count = captured.handoff_direct_lookup_count,
      timeout_evidence_source = JSON_NULL,
    },
    evidence_refs = json_array({
      {
        kind = probe and "runtime-cas-probe" or "runtime-cas-decision",
        ref = probe
          and "devloop.state.versioned_transition_status:" .. tostring(probe.outcome)
          or "devloop.logging.log_cas_decision:" .. tostring(decision.outcome),
      },
      {
        kind = "runtime-event-source",
        ref = tostring(event.payload.source_ref and event.payload.source_ref.ref),
      },
    }),
  }
end

local function capture_guard(fixture, options)
  options = options or {}
  local event = (options.event_factory or fix_reconcile_event)()
  local from_label = options.from_label or FROM_LABEL
  local source_states = options.source_states or SOURCE_STATES
  local comments, head_sha, surface = fixture.fixture(event)
  local decision_from_label = fixture.decision_from_label or from_label
  local result, captured = observe_real_department(
    event,
    comments,
    decision_from_label,
    head_sha,
    surface or options.surface
  )
  t.eq(#captured.probes, fixture.probe_count, fixture.name .. ": writer CAS probe count")
  t.eq(#captured.decisions, 1, fixture.name .. ": one returning decision")
  t.eq(captured.decisions[1].dept, "reconcile", fixture.name .. ": reconcile decision")
  t.eq(captured.decisions[1].from_state, decision_from_label, fixture.name .. ": writer label")
  t.eq(captured.decisions[1].to_state, "blocked", fixture.name .. ": writer target")
  t.eq(captured.decisions[1].outcome, fixture.expected_outcome, fixture.name .. ": exact disposition")
  if fixture.expected_probe ~= nil then
    t.eq(captured.probes[1].outcome, fixture.expected_probe, fixture.name .. ": real CAS regime")
  end
  t.eq(#captured.applies, 0, fixture.name .. ": guard emits no apply")
  t.eq(#captured.raises, 0, fixture.name .. ": guard logs no raise")
  t.eq(#result.raises, 0, fixture.name .. ": runtime emits no effect")
  t.eq(captured.liveness_read_count, 0, fixture.name .. ": no Codex liveness read")
  local record = build_guard_record(
    fixture,
    event,
    result,
    captured,
    options.observation_prefix and options.observation_prefix .. fixture.name or nil
  )
  local expected_source_states = fixture.caller_from_states or source_states
  t.eq(canonical_json(record.old_inputs.caller_from_states), canonical_json(json_array(expected_source_states)), fixture.name .. ": captured source set")
  t.eq(record.old_inputs.target_version, JSON_NULL, fixture.name .. ": four-argument target_version")
  t.eq(record.old_outcome.status, fixture.status, fixture.name .. ": captured status")
  t.eq(record.old_outcome.reason_code, fixture.reason_code, fixture.name .. ": captured reason")
  t.eq(#record.old_outcome.emitted_effects, 0, fixture.name .. ": no emitted effects")
  t.eq(#record.old_outcome.observable_writes, 0, fixture.name .. ": no observable writes")
  return record
end

local function older_version(version)
  local older, replacements = tostring(version):gsub(
    "2026%-06%-03T01%-02%-03Z",
    "2026-06-02T01-02-03Z",
    1
  )
  t.eq(replacements, 1, "fixture version contains the expected timestamp")
  return older
end


local GUARDS = {
  {
    name = "fix-reconcile-marker-visible",
    probe_count = 0,
    status = "skip-idempotent(fix reconcile marker already visible)",
    reason_code = "fix-reconcile-marker-visible",
    expected_outcome = "skip-idempotent(fix reconcile marker already visible)",
    fixture = function(event)
      return json_array({
        core.build_fix_reconcile_comment_request(
          REPO,
          tostring(ISSUE_NUMBER),
          event.payload,
          "drop",
          "already done"
        ).body,
      })
    end,
  },
  {
    name = "already-terminal",
    probe_count = 0,
    status = "skip-idempotent(already terminal)",
    reason_code = "already-terminal",
    expected_outcome = "skip-idempotent(already terminal)",
    fixture = function(event)
      return json_array({
        core.state_marker(event.payload.proposal_id, "blocked", event.payload.issue_version),
      })
    end,
  },
  {
    name = "head-advanced",
    probe_count = 0,
    status = "skip-stale(head-advanced)",
    reason_code = "head-advanced",
    expected_outcome = "skip-stale(head-advanced)",
    fixture = function(event)
      return json_array({
        core.state_marker(event.payload.proposal_id, "reviewing", event.payload.issue_version),
      }), "feedface"
    end,
  },
  {
    name = "incoming-version-older",
    probe_count = 1,
    expected_probe = "stale",
    status = "stale",
    reason_code = "incoming-version-older",
    expected_outcome = "skip-stale(version-mismatch)",
    fixture = function(event)
      local current_version = event.payload.issue_version
      event.payload.issue_version = older_version(current_version)
      event.payload.round = core.version_fix_round(event.payload.issue_version)
      event.payload.dedup_key = "fix-reconcile:" .. event.payload.issue_version
      return json_array({
        core.state_marker(event.payload.proposal_id, "reviewing", current_version),
      })
    end,
  },
  {
    name = "safe-version-mismatch",
    probe_count = 1,
    expected_probe = "apply",
    status = "stale",
    reason_code = "version-mismatch",
    expected_outcome = "skip-stale(version-mismatch)",
    fixture = function(event)
      local current_version = older_version(event.payload.issue_version)
      t.is_true(
        transition_version.compare(event.payload.issue_version, current_version) > 0,
        "safe mismatch fixture keeps incoming version newer"
      )
      return json_array({
        core.state_marker(event.payload.proposal_id, "reviewing", current_version),
      })
    end,
  },
  {
    name = "source-state-mismatch",
    probe_count = 1,
    expected_probe = "stale",
    status = "stale",
    reason_code = "source-state-mismatch",
    expected_outcome = "skip-stale(version-mismatch)",
    fixture = function(event)
      return json_array({
        core.state_marker(event.payload.proposal_id, "awaiting-pr", event.payload.issue_version),
      })
    end,
  },
}

local BOUNDED_GUARDS = {
  GUARDS[1],
  GUARDS[2],
  {
    name = "stale-version-mismatch",
    probe_count = 1,
    expected_probe = "apply",
    status = "stale",
    reason_code = "version-mismatch",
    expected_outcome = "skip-stale(version-mismatch)",
    fixture = function(event)
      local current_version = older_version(event.payload.issue_version)
      t.is_true(
        transition_version.compare(event.payload.issue_version, current_version) > 0,
        "bounded safe mismatch fixture keeps incoming version newer"
      )
      return json_array({
        core.state_marker(event.payload.proposal_id, "fixing", current_version),
      })
    end,
  },
}

local BOUNDED_OPTIONS = {
  event_factory = bounded_fix_reconcile_event,
  from_label = BOUNDED_FROM_LABEL,
  source_states = BOUNDED_SOURCE_STATES,
  observation_prefix = "bounded-fix-",
}

local OWN_CI_GUARDS = {
  GUARDS[1],
  GUARDS[2],
  BOUNDED_GUARDS[3],
  {
    name = "head-advanced",
    probe_count = 0,
    status = "skip-stale(head-advanced)",
    reason_code = "head-advanced",
    expected_outcome = "skip-stale(head-advanced)",
    decision_from_label = "fixing",
    caller_from_states = { "fixing" },
    fixture = function(event)
      return json_array({
        core.state_marker(event.payload.proposal_id, "fixing", event.payload.issue_version),
      }), "feedface", { kind = "own-ci" }
    end,
  },
  {
    name = "pr-closed",
    probe_count = 1,
    expected_probe = "apply",
    status = "skip-stale(pr-closed)",
    reason_code = "pr-closed",
    expected_outcome = "skip-stale(pr-closed)",
    decision_from_label = "fixing",
    fixture = function(event)
      return json_array({
        core.state_marker(event.payload.proposal_id, "fixing", event.payload.issue_version),
      }), nil, { kind = "own-ci", state = "CLOSED" }
    end,
  },
  {
    name = "own-ci-cleared",
    probe_count = 1,
    expected_probe = "apply",
    status = "skip-stale(own-ci-cleared)",
    reason_code = "own-ci-cleared",
    expected_outcome = "skip-stale(own-ci-cleared)",
    decision_from_label = "fixing",
    fixture = function(event)
      return json_array({
        core.state_marker(event.payload.proposal_id, "fixing", event.payload.issue_version),
      }), nil, { kind = "own-ci", conclusion = "SUCCESS" }
    end,
  },
}

local OWN_CI_OPTIONS = {
  event_factory = own_ci_reconcile_event,
  from_label = BOUNDED_FROM_LABEL,
  source_states = BOUNDED_SOURCE_STATES,
  observation_prefix = "own-ci-",
  surface = { kind = "own-ci" },
}

local OWN_CI_POLICY_OPTIONS = {
  event_factory = own_ci_policy_invalid_event,
  from_label = BOUNDED_FROM_LABEL,
  source_states = BOUNDED_SOURCE_STATES,
  observation_prefix = "own-ci-policy-invalid-",
  surface = { kind = "own-ci" },
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
  test_fix_reconcile_old_observations_are_runtime_bound_to_inventory = function()
    local actual = json_array()
    for _, source_state in ipairs(SOURCE_STATES) do
      local ok, record = pcall(capture_apply_record, source_state)
      if not ok then
        error("OLD fix reconcile apply " .. source_state .. " failed: " .. tostring(record), 0)
      end
      table.insert(actual, record)
    end
    for _, fixture in ipairs(GUARDS) do
      local ok, record = pcall(capture_guard, fixture)
      if not ok then
        error("OLD fix reconcile guard " .. fixture.name .. " failed: " .. tostring(record), 0)
      end
      table.insert(actual, record)
    end
    for _, source_state in ipairs(BOUNDED_SOURCE_STATES) do
      local ok, record = pcall(capture_apply_record, source_state, BOUNDED_OPTIONS)
      if not ok then
        error("OLD bounded fix reconcile apply " .. source_state .. " failed: " .. tostring(record), 0)
      end
      table.insert(actual, record)
    end
    for _, fixture in ipairs(BOUNDED_GUARDS) do
      local ok, record = pcall(capture_guard, fixture, BOUNDED_OPTIONS)
      if not ok then
        error("OLD bounded fix reconcile guard " .. fixture.name .. " failed: " .. tostring(record), 0)
      end
      table.insert(actual, record)
    end
    for _, source_state in ipairs(BOUNDED_SOURCE_STATES) do
      local ok, record = pcall(capture_apply_record, source_state, OWN_CI_OPTIONS)
      if not ok then
        error("OLD own-CI reconcile apply " .. source_state .. " failed: " .. tostring(record), 0)
      end
      table.insert(actual, record)
    end
    for _, fixture in ipairs(OWN_CI_GUARDS) do
      local ok, record = pcall(capture_guard, fixture, OWN_CI_OPTIONS)
      if not ok then
        error("OLD own-CI reconcile guard " .. fixture.name .. " failed: " .. tostring(record), 0)
      end
      table.insert(actual, record)
    end
    local policy_ok, policy_record = pcall(capture_apply_record, "fixing", OWN_CI_POLICY_OPTIONS)
    if not policy_ok then
      error("OLD own-CI policy-invalid reconcile apply failed: " .. tostring(policy_record), 0)
    end
    table.insert(actual, policy_record)
    table.sort(actual, function(left, right)
      return tostring(left.observation_id) < tostring(right.observation_id)
    end)

    local expected = committed_records()
    local difference = first_difference(actual, expected, "old_behavior_observations[reconcile_fix]")
    if difference ~= nil or canonical_json(actual) ~= canonical_json(expected) then
      error(
        "runtime-bound OLD fix reconcile observation differs at " .. tostring(difference or "canonical-json")
          .. "; runtime_records=" .. canonical_json(actual),
        0
      )
    end
  end,
}
