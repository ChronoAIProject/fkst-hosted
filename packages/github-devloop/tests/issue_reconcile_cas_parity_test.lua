-- Non-circularity contract: production truth comes from the real issue
-- reconcile department's ordered guards, named CAS probe, and first effect
-- builder admission boundary. Effects and legacy CAS logs are separate axes.
-- This test never computes expected admission with a state transition helper.

local catalog = require("devloop.restart_cas_catalog")
local owner_pending_projection = require("devloop.restart_owner_pending_projection")
local inventories = {
  canonicalization = require("core.restart.canonicalization_inventory"),
  entry = require("core.restart.entry_inventory"),
  operator_reentry = require("core.restart.operator_reentry_inventory"),
}
local devloop_logging = require("devloop.logging")
local devloop_state = require("devloop.state")
local restart_authority = require("core.restart_authority")
local h = require("tests.devloop_helpers")
local t = h.t
local core = h.core
local projection = owner_pending_projection.derive(core.restart_package_name, core.restart_transition_table(), inventories)
local reconcile_department = require("departments.reconcile.main")
local conv_reconcile = require("devloop.convergence.reconcile")

local POLICY_ID = "cas.legacy_issue_reconcile_v1"
local VARIANT = "thinking_to_blocked"
local V_OLDER = "consensus:github-devloop/issue/owner/repo/42/2026-06-02T01-02-03Z"
local V_EQUAL = "consensus:github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z"
local V_NEWER = "consensus:github-devloop/issue/owner/repo/42/2026-06-04T01-02-03Z"
local V_ORDERING_EQUAL_CURRENT = V_EQUAL .. "/loop/01"
local V_ORDERING_EQUAL_INCOMING = V_EQUAL .. "/loop/1"

local function reconcile_event(incoming_version, round)
  local event = h.reconcile()
  event.round = round or 3
  event.base_version = incoming_version
  event.dedup_key = "reconcile:" .. tostring(incoming_version) .. "/loop/" .. tostring(event.round)
  return event
end

-- The reconcile department derives its CAS incoming_version from the CURRENT
-- marker version bumped to the reconcile round (reconcile_terminal_state_version),
-- NOT from event.base_version. The shadow intent builder derives the same version
-- independently from (current_version, round) via the same public helper the
-- department uses, so both deciders see identical CAS inputs without the shadow
-- reading the spied probe.
local function shadow_intent_from_event(event, current_version)
  return {
    semantic_variant = "issue_reconcile_true_stall",
    source_boundary = "devloop_reconcile",
    target = "blocked",
    incoming_version = conv_reconcile.reconcile_terminal_state_version(current_version, event.round),
  }
end

local function seal_snapshot(current_state, current_version)
  return restart_authority.seal_snapshot({
    owner = core.restart_package_name,
    current = {
      state = current_state,
      version = current_version,
    },
  })
end

local function shadow_disposition(decision)
  if decision.status == "apply" then
    return { status = "apply", reason_code = "apply" }
  end
  if decision.status == "idempotent" then
    return { status = "idempotent", reason_code = "already-at-target" }
  end
  if decision.status == "stale" then
    return { status = "stale", reason_code = decision.reason_code }
  end
  error("unexpected shadow status: " .. tostring(decision.status))
end

local function assert_bidirectional(name, observed, shadow)
  t.eq(shadow.status, observed.status, name .. ": shadow status parity")
  t.eq(shadow.reason_code, observed.reason_code, name .. ": shadow reason parity")
end

local function observe_department(run)
  local probes = {}
  local decisions = {}
  local boundary_calls = {}
  local original_versioned = devloop_state.versioned_transition_status
  local original_log_cas = devloop_logging.log_cas_decision
  local original_build_reconcile_comment_request = core.build_reconcile_comment_request

  devloop_state.versioned_transition_status = function(
    current,
    from_states,
    to_state,
    incoming_version,
    target_version
  )
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
    })
    return original_log_cas(dept, proposal_id, current, from_state, to_state, outcome, reason)
  end
  core.build_reconcile_comment_request = function(repo, issue_number, reconcile, action, reason, state_version)
    table.insert(boundary_calls, {
      repo = repo,
      issue_number = issue_number,
      reconcile = reconcile,
      action = action,
      reason = reason,
      state_version = state_version,
    })
    return original_build_reconcile_comment_request(
      repo,
      issue_number,
      reconcile,
      action,
      reason,
      state_version
    )
  end

  local ok, result = pcall(run)
  core.build_reconcile_comment_request = original_build_reconcile_comment_request
  devloop_logging.log_cas_decision = original_log_cas
  devloop_state.versioned_transition_status = original_versioned
  if not ok then
    error(result, 0)
  end
  return result, probes, decisions, boundary_calls
end

local function evidence_from_probe(probe)
  return {
    current = probe.current,
    variant = VARIANT,
    incoming_version = probe.incoming_version,
    target_version = probe.target_version,
  }
end

local function emitted_state(result)
  for _, raised in ipairs(result.raises or {}) do
    if raised.queue == "github-proxy.github_issue_comment_request" then
      return tostring(raised.payload and raised.payload.body or ""):match('state="([^"]+)"')
    end
  end
  return nil
end

local function observed_admission(probe, boundary_reached)
  if boundary_reached then
    return { status = "apply", reason_code = "apply" }
  end
  if probe.outcome == "pending" then
    return { status = "pending", reason_code = "source-marker-not-visible" }
  end
  if probe.outcome == "idempotent" then
    return { status = "idempotent", reason_code = "already-at-target" }
  end
  if probe.outcome == "stale" then
    if tostring(probe.incoming_version or "") ~= tostring(probe.current.version or "") then
      return { status = "stale", reason_code = "incoming-version-older" }
    end
    return { status = "stale", reason_code = "advanced-or-diverged" }
  end
  if probe.outcome ~= "apply" then
    error("issue reconcile admission probe returned an unknown outcome: " .. tostring(probe.outcome))
  end
  error("issue reconcile admission probe applied without reaching the effect boundary")
end

local function post_admission_disposition(result, boundary_reached)
  if not boundary_reached then
    return "not-admitted"
  end
  if emitted_state(result) ~= nil then
    return "effect-emitted(" .. tostring(emitted_state(result)) .. ")"
  end
  return "post-admission-no-effect"
end

local function run_real_department(event)
  local raises = {}
  local original_raise = raise
  raise = function(queue, payload)
    table.insert(raises, { queue = queue, payload = payload })
  end
  local ok, failure = pcall(reconcile_department.pipeline, {
    queue = "devloop_reconcile",
    payload = event,
  })
  raise = original_raise
  return {
    exit_code = ok and 0 or 1,
    error = ok and nil or tostring(failure),
    raises = raises,
  }
end

local function mock_current_issue(event, fixture)
  local comments = {}
  if fixture.current_state ~= nil then
    table.insert(comments, core.state_marker(event.proposal_id, fixture.current_state, fixture.current_version))
  end
  h.mock_bot_env()
  h.mock_issue_reconcile({}, comments)
end

local function assert_catalog_matches_observed_decision(fixture)
  local event = reconcile_event(fixture.incoming_version, fixture.round)
  mock_current_issue(event, fixture)

  local result, probes, decisions, boundary_calls = observe_department(function()
    return run_real_department(event)
  end)

  local admission_phase = #probes == 0 and "pre-cas" or "cas"
  t.eq(admission_phase, fixture.admission_phase or "cas", fixture.name .. ": admission phase")
  t.eq(#probes, admission_phase == "cas" and 1 or 0, fixture.name .. ": real department CAS probe count")
  local probe = probes[1]
  if probe ~= nil then
    t.eq(probe.current.state, fixture.current_state, fixture.name .. ": probe current state")
    t.eq(probe.current.version, fixture.current_version, fixture.name .. ": probe current version")
    t.eq(probe.from_states[1], "thinking", fixture.name .. ": probe source state")
    t.eq(#probe.from_states, 1, fixture.name .. ": probe source state count")
    t.eq(probe.to_state, "blocked", fixture.name .. ": probe target state")
    t.eq(probe.target_version, nil, fixture.name .. ": probe target version")
  end

  t.eq(#decisions, 1, fixture.name .. ": structured CAS decision count")
  local decision = decisions[1]
  t.eq(decision.dept, "reconcile", fixture.name .. ": CAS decision department")
  t.eq(decision.from_state, "thinking", fixture.name .. ": logged source state")
  t.eq(decision.to_state, "blocked", fixture.name .. ": logged target state")
  t.is_true(type(decision.outcome) == "string", fixture.name .. ": legacy log outcome captured")
  t.is_true(type(decision.reason) == "string", fixture.name .. ": legacy log reason captured")

  local boundary_reached = #boundary_calls > 0
  t.eq(#boundary_calls, fixture.boundary_reached and 1 or 0, fixture.name .. ": admission boundary reach")
  if boundary_reached then
    local boundary = boundary_calls[1]
    t.eq(boundary.repo, "owner/repo", fixture.name .. ": boundary repo")
    t.eq(boundary.issue_number, "42", fixture.name .. ": boundary issue")
    t.eq(boundary.reconcile, event, fixture.name .. ": boundary event")
    t.eq(boundary.action, "drop", fixture.name .. ": boundary action")
    t.eq(boundary.state_version, probe.incoming_version, fixture.name .. ": boundary terminal version")
  end

  if probe ~= nil then
    local observed = observed_admission(probe, boundary_reached)
    local evidence = evidence_from_probe(probe)
    t.eq(evidence.current, probe.current, fixture.name .. ": catalog current comes from probe")
    t.eq(evidence.incoming_version, probe.incoming_version, fixture.name .. ": catalog incoming version comes from probe")
    t.eq(evidence.target_version, probe.target_version, fixture.name .. ": catalog target version comes from probe")
    local actual = catalog.resolve(POLICY_ID, evidence, projection)
    t.eq(actual.status, observed.status, fixture.name .. ": admission status parity")
    t.eq(actual.reason_code, observed.reason_code, fixture.name .. ": admission reason parity")
    if fixture.admission_status ~= nil then
      t.eq(observed.status, fixture.admission_status, fixture.name .. ": observed admission status")
      t.eq(actual.status, fixture.admission_status, fixture.name .. ": catalog admission status")
    end
  else
    t.eq(boundary_reached, false, fixture.name .. ": pre-CAS input cannot reach admission boundary")
  end
  t.eq(result.exit_code, fixture.expected_exit_code or 0, fixture.name .. ": department exit code")
  t.eq(#result.raises, fixture.effect_count or 0, fixture.name .. ": captured effect count")
  t.eq(
    post_admission_disposition(result, boundary_reached),
    fixture.post_admission_disposition or "not-admitted",
    fixture.name .. ": post-admission disposition"
  )
  if fixture.legacy_log_outcome ~= nil then
    t.eq(decision.outcome, fixture.legacy_log_outcome, fixture.name .. ": legacy log outcome")
  end
end

local function assert_shadow_matches_observed_decision(fixture)
  local event = reconcile_event(fixture.incoming_version, fixture.round)
  mock_current_issue(event, fixture)

  local _, probes, decisions, boundary_calls = observe_department(function()
    return run_real_department(event)
  end)

  t.eq(#probes, 1, fixture.name .. ": real department CAS probe count")
  local probe = probes[1]
  local boundary_reached = #boundary_calls > 0
  local observed = observed_admission(probe, boundary_reached)
  local shadow_intent = shadow_intent_from_event(event, fixture.current_version)
  local shadow = restart_authority.decide_transition(
    seal_snapshot(fixture.current_state, fixture.current_version),
    shadow_intent
  )
  t.eq(shadow.status ~= "illegal", true, fixture.name .. ": shadow edge supported")
  assert_bidirectional(fixture.name, observed, shadow_disposition(shadow))

  if fixture.expected_status ~= nil then
    t.eq(shadow.status, fixture.expected_status, fixture.name .. ": shadow status")
  end
  if fixture.expected_reason_code ~= nil then
    t.eq(shadow.reason_code, fixture.expected_reason_code, fixture.name .. ": shadow reason")
  end
  t.eq(shadow_intent.incoming_version, probe.incoming_version, fixture.name .. ": independently round-derived version equals the real department's probe input, without reading the spied probe")
  t.eq(#decisions, 1, fixture.name .. ": legacy log still captured once")
end

local function assert_rejected_before_cas(name, payload)
  h.mock_bot_env()
  local result, probes, _, boundary_calls = observe_department(function()
    return run_real_department(payload)
  end)
  t.eq(result.exit_code, 0, name .. ": malformed payload is rejected without a pipeline error")
  t.eq(#probes, 0, name .. ": invalid production input must not reach CAS")
  t.eq(#probes == 0 and "pre-cas" or "cas", "pre-cas", name .. ": admission phase")
  t.eq(#boundary_calls, 0, name .. ": invalid production input must not reach the admission boundary")
end

return {
  test_issue_reconcile_source_equal_applies_at_effect_boundary = function()
    assert_catalog_matches_observed_decision({
      name = "issue-reconcile-source-equal",
      current_state = "thinking",
      current_version = V_EQUAL,
      incoming_version = V_EQUAL,
      boundary_reached = true,
      admission_status = "apply",
      effect_count = 2,
      post_admission_disposition = "effect-emitted(blocked)",
      legacy_log_outcome = "applied",
    })
  end,

  test_issue_reconcile_source_older_compares_catalog_to_observed_admission = function()
    assert_catalog_matches_observed_decision({
      name = "issue-reconcile-source-older",
      current_state = "thinking",
      current_version = V_EQUAL,
      incoming_version = V_OLDER,
      boundary_reached = true,
      admission_status = "apply",
      effect_count = 2,
      post_admission_disposition = "effect-emitted(blocked)",
      legacy_log_outcome = "applied",
    })
  end,

  test_issue_reconcile_source_newer_applies_under_versioned_policy = function()
    assert_catalog_matches_observed_decision({
      name = "issue-reconcile-source-newer",
      current_state = "thinking",
      current_version = V_EQUAL,
      incoming_version = V_NEWER,
      boundary_reached = true,
      admission_status = "apply",
      effect_count = 2,
      post_admission_disposition = "effect-emitted(blocked)",
    })
  end,

  test_issue_reconcile_newer_with_missing_current_is_pending = function()
    assert_catalog_matches_observed_decision({
      name = "issue-reconcile-newer-current-missing",
      current_state = nil,
      current_version = nil,
      incoming_version = V_NEWER,
      admission_phase = "pre-cas",
      expected_exit_code = 1,
      legacy_log_outcome = "pending",
    })
  end,

  test_issue_reconcile_target_state_is_idempotent = function()
    assert_catalog_matches_observed_decision({
      name = "issue-reconcile-target-idempotent",
      current_state = "blocked",
      current_version = V_EQUAL,
      incoming_version = V_EQUAL,
      admission_phase = "pre-cas",
      legacy_log_outcome = "skip-idempotent(already terminal)",
    })
  end,

  test_issue_reconcile_unrelated_current_is_stale = function()
    assert_catalog_matches_observed_decision({
      name = "issue-reconcile-unrelated-current",
      current_state = "ready",
      current_version = V_EQUAL,
      incoming_version = V_EQUAL,
      admission_phase = "pre-cas",
      legacy_log_outcome = "skip-stale(state-advanced)",
    })
  end,

  test_issue_reconcile_ordering_equal_raw_different_has_no_version_overlay = function()
    t.is_true(
      V_ORDERING_EQUAL_CURRENT ~= V_ORDERING_EQUAL_INCOMING,
      "issue-reconcile-ordering-equal-raw-different: fixture versions must be byte-different"
    )
    assert_catalog_matches_observed_decision({
      name = "issue-reconcile-ordering-equal-raw-different",
      current_state = "thinking",
      current_version = V_ORDERING_EQUAL_CURRENT,
      incoming_version = V_ORDERING_EQUAL_INCOMING,
      boundary_reached = true,
      admission_status = "apply",
      effect_count = 2,
      post_admission_disposition = "effect-emitted(blocked)",
    })
  end,

  -- The reconcile department bumps a monotonic round-scoped version and only
  -- reaches the versioned CAS from a live thinking marker, so its sole CAS
  -- outcome is apply. Idempotent (already at/after blocked) and stale
  -- (state-advanced) are resolved by the department's pre-CAS stage_rank guards
  -- and never invoke the probe, so there is no real idempotent/stale CAS
  -- decision to compare the shadow against. incoming_version here only feeds the
  -- reconcile event's base_version (used for reconcile-marker dedup); it does NOT
  -- feed the CAS, whose version is derived from the current marker version bumped
  -- to the reconcile round (reconcile_terminal_state_version).
  test_issue_reconcile_shadow_applies_from_thinking = function()
    assert_shadow_matches_observed_decision({
      name = "issue-reconcile-shadow-apply-from-thinking",
      current_state = "thinking",
      current_version = V_EQUAL,
      incoming_version = V_EQUAL,
      expected_status = "apply",
      expected_reason_code = "apply",
    })
  end,

  test_issue_reconcile_malformed_payload_fails_closed_before_cas = function()
    local payload = reconcile_event(42)
    assert_rejected_before_cas("issue-reconcile-malformed-version", payload)
  end,
}
