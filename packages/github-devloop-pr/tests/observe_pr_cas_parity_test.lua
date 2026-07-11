-- Non-circularity contract: production truth comes from the real observe_pr
-- department's CAS path. The probe, exact guard calls, CAS logs, and effects are
-- observations only. Expected results never call a devloop.state transition
-- helper.

local catalog = require("devloop.restart_cas_catalog")
local devloop_logging = require("devloop.logging")
local m_builders = require("devloop.markers.builders")
local replay_fields = require("devloop.replay_fields")
local replayer = require("devloop.replayer")
local devloop_state = require("devloop.state")
local h = require("tests.devloop_helpers")
local t = h.t
local core = h.core
local observe_pr_department = require("departments.observe_pr.main")

local POLICY_ID = "cas.legacy_observe_pr_v1"
local VARIANT = "pr_open_to_reviewing"
local PROPOSAL_ID = "github-devloop/issue/owner/repo/42"
local BRANCH = "devloop-owner-repo-42-01HY"
local BASE_BRANCH = "dev"
local HEAD_SHA = "def456"
local V_OLDER = "ready/consensus-github-devloop/issue/owner/repo/42/2026-06-02T01-02-03Z"
local V_EQUAL = "ready/consensus-github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z"
local V_NEWER = "ready/consensus-github-devloop/issue/owner/repo/42/2026-06-04T01-02-03Z"
local V_ORDERING_EQUAL_CURRENT = V_EQUAL .. "/loop/01"
local V_ORDERING_EQUAL_INCOMING = V_EQUAL .. "/loop/1"

local function observe_department(run)
  local probes = {}
  local decisions = {}
  local boundary_calls = {}
  local replay_guard_calls = {}
  local closed_guard_calls = {}
  local post_probe_row_calls = {}
  local original_versioned = devloop_state.versioned_transition_status
  local original_log_cas = devloop_logging.log_cas_decision
  local original_restart_transition_row = replay_fields.restart_transition_row
  local original_replay_from_table = replayer.replay_from_table

  devloop_state.versioned_transition_status = function(current, from_states, to_state, incoming_version, target_version)
    local outcome = original_versioned(current, from_states, to_state, incoming_version, target_version)
    table.insert(probes, {
      current = current,
      from_states = from_states,
      to_state = to_state,
      incoming_version = incoming_version,
      target_version = target_version,
      outcome = outcome,
    })
    return outcome
  end
  devloop_logging.log_cas_decision = function(dept, proposal_id, current, from_state, to_state, outcome, reason)
    table.insert(decisions, {
      dept = dept,
      proposal_id = proposal_id,
      current = current,
      from_state = from_state,
      to_state = to_state,
      outcome = outcome,
      reason = reason,
      probe_count = #probes,
    })
    if #probes > 0
      and dept == "observe_pr"
      and proposal_id == PROPOSAL_ID
      and from_state == "pr-open"
      and to_state == "reviewing"
      and outcome == "skip-stale(pr-closed)"
      and reason == "re-derived PR is not open" then
      table.insert(closed_guard_calls, { probe_count = #probes })
    end
    return original_log_cas(dept, proposal_id, current, from_state, to_state, outcome, reason)
  end
  replay_fields.restart_transition_row = function(transition_table, state_name)
    local row = original_restart_transition_row(transition_table, state_name)
    if #probes > 0 then
      table.insert(post_probe_row_calls, {
        state_name = state_name,
        row = row,
        probe_count = #probes,
      })
    end
    return row
  end
  replayer.replay_from_table = function(...)
    local args = { ... }
    local replay_row = args[5]
    -- After the shared probe, the row lookup is either the local-state replay
    -- or maybe_redrive_not_mergeable_pr. Pair the real replay call explicitly;
    -- every unpaired lookup is the downstream admission boundary.
    for index = #post_probe_row_calls, 1, -1 do
      local call = post_probe_row_calls[index]
      if not call.is_replay and call.row == replay_row then
        call.is_replay = true
        table.insert(replay_guard_calls, call)
        break
      end
    end
    return original_replay_from_table(...)
  end

  local ok, result = pcall(run)
  for _, call in ipairs(post_probe_row_calls) do
    if not call.is_replay then
      table.insert(boundary_calls, call)
    end
  end
  replayer.replay_from_table = original_replay_from_table
  replay_fields.restart_transition_row = original_restart_transition_row
  devloop_logging.log_cas_decision = original_log_cas
  devloop_state.versioned_transition_status = original_versioned
  if not ok then
    error(result, 0)
  end
  return result, probes, decisions, boundary_calls, replay_guard_calls, closed_guard_calls
end

local function evidence_from_probe(probe)
  return {
    current = probe.current,
    variant = VARIANT,
    source_states = probe.from_states,
    target_state = probe.to_state,
    incoming_version = probe.incoming_version,
    target_version = probe.target_version,
    -- Production's raw-version overlay compares the same origin version passed
    -- to the observed CAS probe.
    overlay_version = probe.incoming_version,
  }
end

local function pr_event(pr_number, overrides)
  local number = pr_number or 7
  local event = {
    schema = "github-proxy.v1",
    type = "pr",
    repo = "owner/repo",
    number = number,
    state = "OPEN",
    updated_at = "2026-06-04T01:02:03Z",
    dedup_key = "owner/repo#pr#" .. tostring(number) .. "@2026-06-04T01:02:03Z",
    source_ref = {
      kind = "external",
      ref = "owner/repo#pr/" .. tostring(number),
    },
  }
  for key, value in pairs(overrides or {}) do
    event[key] = value
  end
  return event
end

local function run_real_department(event)
  local raises = {}
  local original_raise = raise
  raise = function(queue, payload)
    table.insert(raises, { queue = queue, payload = payload })
  end
  local ok, failure = pcall(observe_pr_department.pipeline, {
    queue = "github-proxy.github_entity_changed",
    payload = event,
  })
  raise = original_raise
  return {
    exit_code = ok and 0 or 1,
    error = ok and nil or tostring(failure),
    raises = raises,
  }
end

local function emitted_state(result)
  for _, raised in ipairs(result.raises or {}) do
    if raised.queue == "github-proxy.github_pr_comment_request" then
      return tostring(raised.payload and raised.payload.body or ""):match('state="([^"]+)"')
    end
  end
  return nil
end

local function primary_decision(decisions, probe_reached)
  for _, decision in ipairs(decisions) do
    if decision.dept == "observe_pr"
      and decision.proposal_id == PROPOSAL_ID
      and ((probe_reached and decision.probe_count > 0)
        or (not probe_reached and decision.from_state == "reviewing")) then
      return decision
    end
  end
  return nil
end

local function decision_summary(decisions)
  local out = {}
  for _, decision in ipairs(decisions) do
    table.insert(out, table.concat({
      tostring(decision.proposal_id),
      tostring(decision.from_state) .. "->" .. tostring(decision.to_state),
      tostring(decision.outcome),
      tostring(decision.reason),
    }, ":"))
  end
  return table.concat(out, " | ")
end

local function observed_admission(fixture, probe, decision, admitted_guard_reached)
  if probe == nil then
    return { status = "pre-cas", reason_code = "cas-probe-not-reached" }
  end
  if probe.outcome == "pending" then
    return { status = "pending", reason_code = "source-marker-not-visible" }
  end
  if probe.outcome == "idempotent" then
    return { status = "idempotent", reason_code = "already-at-target" }
  end
  if probe.outcome == "stale" then
    return { status = "stale", reason_code = "incoming-version-older" }
  end
  if probe.outcome ~= "apply" then
    error(fixture.name .. ": observe_pr CAS probe returned " .. tostring(probe.outcome))
  end

  local legacy_reason = tostring(decision and decision.reason or "")
  if not admitted_guard_reached and legacy_reason:find("state") ~= nil then
    return { status = "stale", reason_code = "from-state-mismatch" }
  end
  if not admitted_guard_reached and legacy_reason:find("version") ~= nil then
    return { status = "stale", reason_code = "version-mismatch" }
  end
  if admitted_guard_reached then
    return { status = "apply", reason_code = "apply" }
  end
  error(fixture.name .. ": observe_pr admission apply did not reach a classified guard")
end

local function post_admission_disposition(result, boundary_reached, pre_builder_admission_reached)
  local state = emitted_state(result)
  local admitted_guard_reached = boundary_reached or pre_builder_admission_reached
  if not admitted_guard_reached then
    if state ~= nil then
      return "effect-replayed(" .. state .. ")"
    end
    return "not-admitted"
  end
  if state ~= nil then
    if pre_builder_admission_reached then
      return "effect-replayed(" .. state .. ")"
    end
    return "effect-emitted(" .. state .. ")"
  end
  return "admitted-no-effect"
end

local function assert_observe_pr_admission_case(fixture)
  local comments = {
    m_builders.pr_origin_marker(PROPOSAL_ID, "42", BRANCH, fixture.incoming_version, BASE_BRANCH),
  }
  if fixture.current_state ~= nil then
    table.insert(comments, core.state_marker(PROPOSAL_ID, fixture.current_state, fixture.current_version))
  end

  h.mock_bot_env()
  h.mock_default_issue_claim("owner/repo", 42)
  h.mock_pr_origin_for({
    number = fixture.pr_number,
    comments = comments,
    head = BRANCH,
    head_sha = HEAD_SHA,
    state = fixture.pr_state or "OPEN",
    base_branch = BASE_BRANCH,
    times = 2,
  })

  local result, probes, decisions, boundary_calls, replay_guard_calls, closed_guard_calls = observe_department(function()
    return run_real_department(pr_event(fixture.pr_number))
  end)

  t.eq(
    result.exit_code,
    fixture.expected_exit_code or 0,
    fixture.name .. ": department exit code: " .. tostring(result.error or "ok")
  )
  local expected_probe_count = fixture.probe_reached == false and 0 or 1
  t.eq(
    #probes,
    expected_probe_count,
    fixture.name .. ": real department CAS probe count; decisions=" .. decision_summary(decisions)
  )
  local probe = probes[1]
  if probe ~= nil then
    t.is_true(type(probe.current) == "table", fixture.name .. ": probe current fact")
    t.eq(#probe.from_states, 2, fixture.name .. ": probe source state count")
    t.eq(probe.from_states[1], "pr-open", fixture.name .. ": first probe source state")
    t.eq(probe.from_states[2], "unmanaged", fixture.name .. ": second probe source state")
    t.eq(probe.to_state, "reviewing", fixture.name .. ": probe target state")
  end

  local boundary_reached = #boundary_calls > 0
  t.eq(#boundary_calls, fixture.boundary_reached and 1 or 0, fixture.name .. ": admission boundary reach")
  if boundary_reached then
    t.eq(boundary_calls[1].state_name, fixture.current_state, fixture.name .. ": boundary state")
    t.eq(boundary_calls[1].probe_count, 1, fixture.name .. ": boundary follows the CAS probe")
  end
  local pre_builder_admission_reached = #closed_guard_calls > 0
  for _, call in ipairs(replay_guard_calls) do
    if call.probe_count > 0 then
      pre_builder_admission_reached = true
    end
  end
  t.eq(
    pre_builder_admission_reached,
    fixture.pre_builder_admission_reached == true,
    fixture.name .. ": admitted-before-builder guard reach"
  )
  local admitted_guard_reached = boundary_reached or pre_builder_admission_reached
  if admitted_guard_reached then
    t.eq(probe and probe.outcome, "apply", fixture.name .. ": admission boundary requires an applied probe")
  end

  local decision = primary_decision(decisions, probe ~= nil)
  t.is_true(
    decision ~= nil,
    fixture.name .. ": structured CAS decision captured; decisions=" .. decision_summary(decisions)
  )
  t.eq(decision.dept, "observe_pr", fixture.name .. ": CAS decision department")
  t.is_true(type(decision.outcome) == "string", fixture.name .. ": legacy log outcome captured")
  t.is_true(type(decision.reason) == "string", fixture.name .. ": legacy log reason captured")

  local observed = observed_admission(fixture, probe, decision, admitted_guard_reached)
  local actual = nil
  if probe ~= nil then
    actual = catalog.resolve(POLICY_ID, evidence_from_probe(probe))
    t.eq(actual.status, observed.status, fixture.name .. ": admission status parity")
    t.eq(actual.reason_code, observed.reason_code, fixture.name .. ": admission reason parity")
  end
  if fixture.probe_outcome ~= nil then
    t.eq(probe and probe.outcome, fixture.probe_outcome, fixture.name .. ": literal probe outcome")
  end
  if fixture.admission_status ~= nil then
    t.eq(observed.status, fixture.admission_status, fixture.name .. ": observed admission status")
    if actual ~= nil then
      t.eq(actual.status, fixture.admission_status, fixture.name .. ": catalog admission status")
    else
      t.eq(fixture.admission_status, "pre-cas", fixture.name .. ": zero-probe classification")
    end
  end
  if fixture.admission_reason_code ~= nil then
    t.eq(observed.reason_code, fixture.admission_reason_code, fixture.name .. ": observed admission reason")
    if actual ~= nil then
      t.eq(actual.reason_code, fixture.admission_reason_code, fixture.name .. ": catalog admission reason")
    end
  end
  t.eq(
    post_admission_disposition(result, boundary_reached, pre_builder_admission_reached),
    fixture.post_admission_disposition or "not-admitted",
    fixture.name .. ": post-admission disposition"
  )
  if fixture.legacy_log_outcome ~= nil then
    t.eq(decision.outcome, fixture.legacy_log_outcome, fixture.name .. ": legacy log outcome")
  end
  if fixture.effect_state ~= nil then
    t.eq(emitted_state(result), fixture.effect_state, fixture.name .. ": emitted effect target")
  else
    t.eq(emitted_state(result), nil, fixture.name .. ": no admitted state effect")
  end
end

local function assert_malformed_event_is_pre_cas()
  local malformed = pr_event(712, {
    number = "not-a-pr-number",
    source_ref = {
      kind = "external",
      ref = "owner/repo#pr/not-a-pr-number",
    },
  })
  local result, probes, decisions, boundary_calls = observe_department(function()
    return run_real_department(malformed)
  end)

  t.eq(result.exit_code, 0, "observe-pr-malformed: department rejects unsupported payload")
  t.eq(#probes, 0, "observe-pr-malformed: rejected input does not reach the CAS probe")
  t.eq(#boundary_calls, 0, "observe-pr-malformed: rejected input does not reach admission boundary")
  t.eq(#result.raises, 0, "observe-pr-malformed: rejected input emits no effect")
  t.eq(#decisions, 1, "observe-pr-malformed: rejection decision count")
  t.eq(decisions[1].outcome, "skip-foreign(pr)", "observe-pr-malformed: rejection outcome")
  t.eq(decisions[1].reason, "unsupported event payload", "observe-pr-malformed: rejection reason")
end

return {
  test_observe_pr_source_equal_applies = function()
    assert_observe_pr_admission_case({
      name = "observe-pr-source-equal",
      pr_number = 701,
      current_state = "pr-open",
      current_version = V_EQUAL,
      incoming_version = V_EQUAL,
      probe_outcome = "apply",
      boundary_reached = true,
      admission_status = "apply",
      admission_reason_code = "apply",
      effect_state = "reviewing",
      post_admission_disposition = "effect-emitted(reviewing)",
      legacy_log_outcome = "applied",
    })
  end,

  test_observe_pr_unmanaged_source_applies = function()
    assert_observe_pr_admission_case({
      name = "observe-pr-unmanaged-source",
      pr_number = 702,
      current_state = nil,
      current_version = nil,
      incoming_version = V_EQUAL,
      probe_outcome = "apply",
      boundary_reached = true,
      admission_status = "apply",
      admission_reason_code = "apply",
      effect_state = "reviewing",
      post_admission_disposition = "effect-emitted(reviewing)",
      legacy_log_outcome = "applied",
    })
  end,

  test_observe_pr_source_older_is_stale = function()
    assert_observe_pr_admission_case({
      name = "observe-pr-source-older",
      pr_number = 703,
      current_state = "pr-open",
      current_version = V_EQUAL,
      incoming_version = V_OLDER,
      probe_outcome = "stale",
      admission_status = "stale",
      admission_reason_code = "incoming-version-older",
    })
  end,

  test_observe_pr_source_newer_is_stale_raw_version_mismatch = function()
    assert_observe_pr_admission_case({
      name = "observe-pr-source-newer",
      pr_number = 704,
      current_state = "pr-open",
      current_version = V_EQUAL,
      incoming_version = V_NEWER,
      probe_outcome = "apply",
      admission_status = "stale",
      admission_reason_code = "version-mismatch",
      legacy_log_outcome = "skip-stale(version-mismatch)",
    })
  end,

  test_observe_pr_ordering_equal_raw_different_is_stale_version_mismatch = function()
    t.is_true(
      V_ORDERING_EQUAL_CURRENT ~= V_ORDERING_EQUAL_INCOMING,
      "observe-pr-ordering-equal-raw-different: fixture versions must be byte-different"
    )
    assert_observe_pr_admission_case({
      name = "observe-pr-ordering-equal-raw-different",
      pr_number = 705,
      current_state = "pr-open",
      current_version = V_ORDERING_EQUAL_CURRENT,
      incoming_version = V_ORDERING_EQUAL_INCOMING,
      probe_outcome = "apply",
      admission_status = "stale",
      admission_reason_code = "version-mismatch",
      legacy_log_outcome = "skip-stale(version-mismatch)",
    })
  end,

  test_observe_pr_target_state_is_pre_cas = function()
    assert_observe_pr_admission_case({
      name = "observe-pr-target-idempotent",
      pr_number = 706,
      current_state = "reviewing",
      current_version = V_EQUAL,
      incoming_version = V_EQUAL,
      probe_reached = false,
      admission_status = "pre-cas",
      admission_reason_code = "cas-probe-not-reached",
      legacy_log_outcome = "skip-idempotent(already at to_state)",
      effect_state = "reviewing",
      post_admission_disposition = "effect-replayed(reviewing)",
    })
  end,

  test_observe_pr_unrelated_current_is_pre_cas = function()
    assert_observe_pr_admission_case({
      name = "observe-pr-unrelated-stale",
      pr_number = 707,
      current_state = "blocked",
      current_version = V_EQUAL,
      incoming_version = V_EQUAL,
      probe_reached = false,
      admission_status = "pre-cas",
      admission_reason_code = "cas-probe-not-reached",
      -- The replay branch's legacy log calls every non-pr-open marker
      -- idempotent. Pre-CAS classification deliberately keeps that observation
      -- separate from catalog parity.
      legacy_log_outcome = "skip-idempotent(already at to_state)",
    })
  end,

  test_observe_pr_closed_source_is_admitted_before_downstream_guard = function()
    assert_observe_pr_admission_case({
      name = "observe-pr-closed-source-admitted",
      pr_number = 708,
      current_state = "pr-open",
      current_version = V_EQUAL,
      incoming_version = V_EQUAL,
      pr_state = "CLOSED",
      probe_outcome = "apply",
      boundary_reached = false,
      pre_builder_admission_reached = true,
      admission_status = "apply",
      admission_reason_code = "apply",
      effect_state = "closed-unmerged",
      post_admission_disposition = "effect-replayed(closed-unmerged)",
    })
  end,

  test_observe_pr_closed_unmanaged_is_admitted_before_downstream_guard = function()
    assert_observe_pr_admission_case({
      name = "observe-pr-closed-unmanaged-admitted",
      pr_number = 709,
      current_state = nil,
      current_version = nil,
      incoming_version = V_EQUAL,
      pr_state = "CLOSED",
      probe_outcome = "apply",
      boundary_reached = false,
      pre_builder_admission_reached = true,
      admission_status = "apply",
      admission_reason_code = "apply",
      post_admission_disposition = "admitted-no-effect",
      legacy_log_outcome = "skip-stale(pr-closed)",
    })
  end,

  test_observe_pr_target_state_with_older_event_is_pre_cas = function()
    assert_observe_pr_admission_case({
      name = "observe-pr-target-older",
      pr_number = 710,
      current_state = "reviewing",
      current_version = V_EQUAL,
      incoming_version = V_OLDER,
      probe_reached = false,
      admission_status = "pre-cas",
      admission_reason_code = "cas-probe-not-reached",
      legacy_log_outcome = "skip-idempotent(already at to_state)",
      effect_state = "reviewing",
      post_admission_disposition = "effect-replayed(reviewing)",
    })
  end,

  test_observe_pr_unrelated_state_with_older_event_is_pre_cas = function()
    assert_observe_pr_admission_case({
      name = "observe-pr-unrelated-older",
      pr_number = 711,
      current_state = "blocked",
      current_version = V_EQUAL,
      incoming_version = V_OLDER,
      probe_reached = false,
      admission_status = "pre-cas",
      admission_reason_code = "cas-probe-not-reached",
      legacy_log_outcome = "skip-idempotent(already at to_state)",
    })
  end,

  test_observe_pr_malformed_event_is_rejected_before_cas = function()
    assert_malformed_event_is_pre_cas()
  end,
}
