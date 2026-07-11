-- Non-circularity contract: production truth comes from the real timeout
-- reconcile department's named CAS probe and first effect-builder admission
-- boundary. Catalog evidence is copied from observed probe arguments, never
-- reconstructed from fixture fields. Pre-CAS guards, effects, and legacy CAS
-- logs are recorded as separate axes.

local catalog = require("devloop.restart_cas_catalog")
local conv_reconcile = require("devloop.convergence.reconcile")
local devloop_logging = require("devloop.logging")
local devloop_state = require("devloop.state")
local transition_version = require("contract.transition_version")
local h = require("tests.devloop_helpers")
local t = h.t
local core = h.core
local reconcile_department = require("departments.reconcile.main")

local POLICY_ID = "cas.legacy_timeout_reconcile_v1"
local PROPOSAL_ID = "github-devloop/issue/owner/repo/42"
local SOURCE_REF = { kind = "external", ref = "owner/repo#issue/42" }
local V_OLDER = "ready/consensus-github-devloop/issue/owner/repo/42/2026-06-02T01-02-03Z"
local V_EQUAL = "ready/consensus-github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z"
local V_NEWER = "ready/consensus-github-devloop/issue/owner/repo/42/2026-06-04T01-02-03Z"
local READY_ATTEMPT = V_EQUAL .. "/timeout/ready/3"
local V_ORDERING_EQUAL_CURRENT = V_EQUAL .. "/loop/01/timeout/ready/3"
local V_ORDERING_EQUAL_EVENT = V_EQUAL .. "/loop/1/timeout/ready/3"

local variants = {
  thinking = "thinking_to_blocked",
  ready = "ready_to_blocked",
  implementing = "implementing_to_blocked",
  reviewing = "reviewing_to_blocked",
  fixing = "fixing_to_blocked",
  ["merge-ready"] = "merge_ready_to_blocked",
  merging = "merging_to_blocked",
}

local function timeout_event(state_name, issue_version, round)
  local n = round or 3
  return {
    schema = "github-devloop.timeout-reconcile.v1",
    proposal_id = PROPOSAL_ID,
    state = state_name,
    issue_version = issue_version,
    round = n,
    dedup_key = "timeout-reconcile:" .. tostring(issue_version)
      .. "/timeout-reconcile/" .. tostring(state_name) .. "/" .. tostring(n),
    source_ref = SOURCE_REF,
  }
end

local function trusted_comment(body, created_at)
  return {
    body = body,
    author_login = core._test_bot_login,
    created_at = created_at or "2026-06-03T00:00:00Z",
  }
end

local function observe_department(run)
  local probes = {}
  local decisions = {}
  local boundary_calls = {}
  local original_versioned = devloop_state.versioned_transition_status
  local original_log_cas = devloop_logging.log_cas_decision
  local original_build_timeout = conv_reconcile.build_timeout_reconcile_comment_request

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
  conv_reconcile.build_timeout_reconcile_comment_request = function(
    repo,
    issue_number,
    reconcile,
    action,
    reason,
    version,
    fields
  )
    table.insert(boundary_calls, {
      repo = repo,
      issue_number = issue_number,
      reconcile = reconcile,
      action = action,
      reason = reason,
      version = version,
      fields = fields,
    })
    return original_build_timeout(repo, issue_number, reconcile, action, reason, version, fields)
  end

  local ok, result = pcall(run)
  conv_reconcile.build_timeout_reconcile_comment_request = original_build_timeout
  devloop_logging.log_cas_decision = original_log_cas
  devloop_state.versioned_transition_status = original_versioned
  if not ok then
    error(result, 0)
  end
  return result, probes, decisions, boundary_calls
end

local function probe_variant(probe)
  if type(probe.from_states) ~= "table" or #probe.from_states ~= 1 or probe.to_state ~= "blocked" then
    return nil
  end
  return variants[probe.from_states[1]]
end

local function evidence_from_probe(probe)
  local variant_name = probe_variant(probe)
  local definition = catalog.definition(POLICY_ID)
  local variant = definition and definition.variants[variant_name]
  t.is_true(variant ~= nil, "observed timeout reconcile probe must select a catalog variant")
  t.eq(#variant.source_states, #probe.from_states, "catalog source-state count comes from probe signature")
  for index, source_state in ipairs(probe.from_states) do
    t.eq(variant.source_states[index], source_state, "catalog source state comes from probe signature")
  end
  t.eq(variant.target_state, probe.to_state, "catalog target state comes from probe signature")
  return {
    current = probe.current,
    variant = variant_name,
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
    error("timeout reconcile admission probe returned an unknown outcome: " .. tostring(probe.outcome))
  end
  error("timeout reconcile admission probe applied without reaching the effect boundary")
end

local function post_admission_disposition(result, boundary_reached)
  if not boundary_reached then
    return "not-admitted"
  end
  local state_name = emitted_state(result)
  if state_name ~= nil then
    return "effect-emitted(" .. state_name .. ")"
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
    queue = "devloop_timeout_reconcile",
    payload = event,
  })
  raise = original_raise
  return {
    exit_code = ok and 0 or 1,
    error = ok and nil or tostring(failure),
    raises = raises,
  }
end

local function fixture_comments(event, fixture)
  local comments = {}
  if fixture.current_state ~= nil then
    table.insert(comments, trusted_comment(core.state_marker(
      PROPOSAL_ID,
      fixture.current_state,
      fixture.current_version
    )))
  end
  if fixture.result_marker_visible then
    table.insert(comments, trusted_comment(conv_reconcile.timeout_reconcile_marker(
      PROPOSAL_ID,
      event.issue_version,
      event.state,
      event.round,
      "drop",
      { source_ref = SOURCE_REF }
    )))
  end
  return comments
end

local function assert_case(fixture)
  local event = fixture.event or timeout_event(
    fixture.event_state or "ready",
    fixture.event_version or fixture.current_version,
    fixture.round
  )
  h.mock_bot_env()
  if fixture.mock_issue ~= false then
    h.mock_issue_reconcile({}, fixture_comments(event, fixture))
  end

  local result, probes, decisions, boundary_calls = observe_department(function()
    return run_real_department(event)
  end)

  local admission_phase = #probes == 0 and "pre-cas" or "cas"
  t.eq(admission_phase, fixture.admission_phase or "cas", fixture.name .. ": admission phase")
  t.eq(#probes, admission_phase == "cas" and 1 or 0, fixture.name .. ": production CAS probe count")
  t.eq(#decisions, 1, fixture.name .. ": legacy CAS decision count")
  local decision = decisions[1]
  t.eq(decision.dept, "reconcile", fixture.name .. ": CAS decision department")
  t.is_true(type(decision.outcome) == "string", fixture.name .. ": legacy log outcome captured")
  t.is_true(type(decision.reason) == "string", fixture.name .. ": legacy log reason captured")

  local boundary_reached = #boundary_calls > 0
  t.eq(#boundary_calls, fixture.boundary_reached and 1 or 0, fixture.name .. ": admission boundary reach")
  if admission_phase == "cas" then
    local probe = probes[1]
    t.eq(probe.from_states[1], event.state, fixture.name .. ": probe source state")
    t.eq(#probe.from_states, 1, fixture.name .. ": probe source-state count")
    t.eq(probe.to_state, "blocked", fixture.name .. ": probe target state")
    t.eq(probe.target_version, nil, fixture.name .. ": probe target version")

    local observed = observed_admission(probe, boundary_reached)
    local evidence = evidence_from_probe(probe)
    t.eq(evidence.current, probe.current, fixture.name .. ": catalog current comes from probe")
    t.eq(evidence.incoming_version, probe.incoming_version, fixture.name .. ": catalog incoming comes from probe")
    t.eq(evidence.target_version, probe.target_version, fixture.name .. ": catalog target comes from probe")
    local actual = catalog.resolve(POLICY_ID, evidence)
    t.eq(actual.status, observed.status, fixture.name .. ": admission status parity")
    t.eq(actual.reason_code, observed.reason_code, fixture.name .. ": admission reason parity")
    if fixture.admission_status ~= nil then
      t.eq(observed.status, fixture.admission_status, fixture.name .. ": observed admission status")
      t.eq(actual.status, fixture.admission_status, fixture.name .. ": catalog admission status")
    end
    if fixture.probe_incoming_is_derived then
      t.is_true(
        probe.incoming_version ~= event.issue_version,
        fixture.name .. ": probe incoming is production-derived, not the raw event version"
      )
    end
  else
    t.eq(boundary_reached, false, fixture.name .. ": pre-CAS input cannot reach admission boundary")
  end

  if boundary_reached then
    local boundary = boundary_calls[1]
    local probe = probes[1]
    t.eq(boundary.repo, "owner/repo", fixture.name .. ": boundary repo")
    t.eq(boundary.issue_number, "42", fixture.name .. ": boundary issue")
    t.eq(boundary.reconcile, event, fixture.name .. ": boundary event")
    t.eq(boundary.action, "drop", fixture.name .. ": boundary action")
    t.eq(boundary.version, probe.incoming_version, fixture.name .. ": boundary version is probe incoming")
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

return {
  test_timeout_reconcile_source_applies_at_effect_builder_boundary = function()
    assert_case({
      name = "timeout-reconcile-source-apply",
      current_state = "ready",
      current_version = READY_ATTEMPT,
      boundary_reached = true,
      admission_status = "apply",
      probe_incoming_is_derived = true,
      effect_count = 2,
      post_admission_disposition = "effect-emitted(blocked)",
      legacy_log_outcome = "applied",
    })
  end,

  test_timeout_reconcile_safe_equal_raw_different_uses_observed_probe_evidence = function()
    t.is_true(
      V_ORDERING_EQUAL_CURRENT ~= V_ORDERING_EQUAL_EVENT,
      "timeout-reconcile-safe-equal: fixture versions must be byte-different"
    )
    t.eq(
      transition_version.strip_suffixes(V_ORDERING_EQUAL_CURRENT),
      transition_version.strip_suffixes(V_ORDERING_EQUAL_EVENT),
      "timeout-reconcile-safe-equal: fixture versions must share canonical lineage"
    )
    assert_case({
      name = "timeout-reconcile-safe-equal-raw-different",
      current_state = "ready",
      current_version = V_ORDERING_EQUAL_CURRENT,
      event_version = V_ORDERING_EQUAL_EVENT,
      boundary_reached = true,
      admission_status = "apply",
      probe_incoming_is_derived = true,
      effect_count = 2,
      post_admission_disposition = "effect-emitted(blocked)",
      legacy_log_outcome = "applied",
    })
  end,

  test_timeout_reconcile_visible_result_marker_is_pre_cas_effect_idempotency = function()
    assert_case({
      name = "timeout-reconcile-result-visible",
      current_state = "ready",
      current_version = READY_ATTEMPT,
      result_marker_visible = true,
      admission_phase = "pre-cas",
      legacy_log_outcome = "skip-idempotent(timeout reconcile marker already visible)",
    })
  end,

  test_timeout_reconcile_target_current_is_pre_cas_from_state_stale = function()
    assert_case({
      name = "timeout-reconcile-target-current",
      current_state = "blocked",
      current_version = READY_ATTEMPT,
      event_state = "ready",
      event_version = READY_ATTEMPT,
      admission_phase = "pre-cas",
      legacy_log_outcome = "skip-stale(state-advanced)",
    })
  end,

  test_timeout_reconcile_terminal_current_is_pre_cas_idempotent = function()
    assert_case({
      name = "timeout-reconcile-terminal-current",
      current_state = "merged",
      current_version = READY_ATTEMPT,
      event_state = "ready",
      event_version = READY_ATTEMPT,
      admission_phase = "pre-cas",
      legacy_log_outcome = "skip-idempotent(already terminal)",
    })
  end,

  test_timeout_reconcile_older_event_is_pre_cas_lineage_stale = function()
    assert_case({
      name = "timeout-reconcile-older-event",
      current_state = "ready",
      current_version = READY_ATTEMPT,
      event_version = V_OLDER .. "/timeout/ready/3",
      admission_phase = "pre-cas",
      legacy_log_outcome = "skip-stale(lineage-mismatch)",
    })
  end,

  test_timeout_reconcile_newer_event_is_pre_cas_lineage_stale = function()
    assert_case({
      name = "timeout-reconcile-newer-event",
      current_state = "ready",
      current_version = READY_ATTEMPT,
      event_version = V_NEWER .. "/timeout/ready/3",
      admission_phase = "pre-cas",
      legacy_log_outcome = "skip-stale(lineage-mismatch)",
    })
  end,

  test_timeout_reconcile_unrelated_current_is_pre_cas_stale = function()
    assert_case({
      name = "timeout-reconcile-unrelated-current",
      current_state = "thinking",
      current_version = READY_ATTEMPT,
      event_state = "ready",
      event_version = READY_ATTEMPT,
      admission_phase = "pre-cas",
      legacy_log_outcome = "skip-stale(state-advanced)",
    })
  end,

  test_timeout_reconcile_missing_current_is_pre_cas_pending = function()
    assert_case({
      name = "timeout-reconcile-current-missing",
      current_state = nil,
      current_version = nil,
      event_state = "ready",
      event_version = READY_ATTEMPT,
      admission_phase = "pre-cas",
      expected_exit_code = 1,
      legacy_log_outcome = "pending",
    })
  end,

  test_timeout_reconcile_malformed_payload_fails_closed_before_cas = function()
    local event = timeout_event("ready", 42)
    assert_case({
      name = "timeout-reconcile-malformed-version",
      event = event,
      mock_issue = false,
      admission_phase = "pre-cas",
      legacy_log_outcome = "skip-foreign(proposal_id)",
    })
  end,
}
