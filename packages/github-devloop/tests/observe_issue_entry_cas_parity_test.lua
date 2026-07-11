-- Non-circularity contract: production truth comes from the real observe_issue
-- department's CAS probe and the issue-claim admission boundary. Effects and
-- legacy CAS logs are recorded as separate post-admission observations. This
-- test never computes the expected result with a devloop.state transition helper.
-- Inputs that never reach the probe are pre-CAS admissions and are not resolved
-- through the catalog.

local catalog = require("devloop.restart_cas_catalog")
local devloop_claims = require("devloop.claims")
local devloop_logging = require("devloop.logging")
local devloop_state = require("devloop.state")
local dispatch_live_run = require("devloop.dispatch_live_run")
local h = require("tests.devloop_helpers")
local t = h.t
local core = h.core
local observe_issue_department = require("departments.observe_issue.main")

local POLICY_ID = "cas.legacy_observe_issue_entry_v1"
local VARIANT = "unmanaged_to_thinking"
local V_OLDER = "github-devloop/issue/owner/repo/42/2026-06-02T01-02-03Z"
local V_EQUAL = "github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z"
local V_NEWER = "github-devloop/issue/owner/repo/42/2026-06-04T01-02-03Z"
local V_ORDERING_EQUAL_CURRENT = V_EQUAL .. "/loop/01"
local V_ORDERING_EQUAL_INCOMING = V_EQUAL .. "/loop/1"

local function observe_department(run)
  local probes = {}
  local decisions = {}
  local boundary_calls = {}
  local original_versioned = devloop_state.versioned_transition_status
  local original_log_cas = devloop_logging.log_cas_decision
  local original_claim_issue = devloop_claims.claim_issue_for_management
  -- A fresh fixture has no live codex run. Force that ground truth so managed
  -- pre-CAS replay checks cannot inherit a cross-case live-run registry entry.
  local original_dispatch_live_run_dedup = dispatch_live_run.dispatch_live_run_dedup
  dispatch_live_run.dispatch_live_run_dedup = function()
    return false
  end

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
    })
    return original_log_cas(dept, proposal_id, current, from_state, to_state, outcome, reason)
  end
  devloop_claims.claim_issue_for_management = function(M, dept, repo, issue_number, current, proposal_id)
    local outcome = original_claim_issue(M, dept, repo, issue_number, current, proposal_id)
    table.insert(boundary_calls, {
      dept = dept,
      repo = repo,
      issue_number = issue_number,
      current = current,
      proposal_id = proposal_id,
      outcome = outcome,
    })
    return outcome
  end

  local ok, result = pcall(run)
  dispatch_live_run.dispatch_live_run_dedup = original_dispatch_live_run_dedup
  devloop_claims.claim_issue_for_management = original_claim_issue
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
    source_states = probe.from_states,
    target_state = probe.to_state,
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
    error("observe-issue-entry admission probe returned an unknown outcome: " .. tostring(probe.outcome))
  end
  if boundary_reached then
    return { status = "apply", reason_code = "apply" }
  end
  error("observe-issue-entry admission apply did not reach the issue-claim guard")
end

local function post_admission_disposition(result, decision, boundary_reached)
  if not boundary_reached then
    return "not-admitted"
  end
  local state = emitted_state(result)
  if state ~= nil then
    return "effect-emitted(" .. state .. ")"
  end
  local outcome = tostring(decision and decision.outcome or "")
  if outcome:find("claim", 1, true) ~= nil then
    return "claim-deferred"
  end
  return "post-admission-no-effect"
end

local function run_real_department(event)
  local raises = {}
  local original_raise = raise
  raise = function(queue, payload)
    table.insert(raises, { queue = queue, payload = payload })
  end
  local ok, failure = pcall(observe_issue_department.pipeline, {
    queue = "devloop_observe_issue",
    payload = event,
    ts = "2026-06-03T01:02:04Z",
  })
  raise = original_raise
  return {
    exit_code = ok and 0 or 1,
    error = ok and nil or tostring(failure),
    raises = raises,
  }
end

local function state_comment(proposal_id, state, version)
  return {
    body = core.state_marker(proposal_id, state, version),
    author_login = "fkst-test-bot",
    created_at = os.date("!%Y-%m-%dT%H:%M:%SZ", now()),
  }
end

local function mock_current_issue(event, fixture)
  local proposal_id = "github-devloop/issue/owner/repo/42"
  local comments = {}
  if fixture.current_state ~= nil then
    table.insert(comments, state_comment(proposal_id, fixture.current_state, fixture.current_version))
  end
  h.mock_issue_state({ "fkst-dev:enabled" }, "OPEN", comments)
  h.mock_context_bundle(event)
end

local function assert_catalog_matches_observed_decision(fixture)
  local event = h.issue({ dedup_key = fixture.incoming_version })
  mock_current_issue(event, fixture)

  local result, probes, decisions, boundary_calls = observe_department(function()
    return run_real_department(event)
  end)

  t.is_true(#probes <= 1, fixture.name .. ": real department CAS probe count must be at most one")
  local boundary_reached = #boundary_calls > 0
  if #probes == 0 then
    t.eq(
      result.exit_code,
      fixture.expected_exit_code or 0,
      fixture.name .. ": pre-CAS department exit code: " .. tostring(result.error)
    )
    t.eq(#boundary_calls, 0, fixture.name .. ": pre-CAS input must not reach the admission boundary")
    local decision = decisions[#decisions]
    local disposition = post_admission_disposition(result, decision, false)
    t.eq(disposition, fixture.post_admission_disposition or "not-admitted", fixture.name .. ": pre-CAS disposition")
    return "pre-cas"
  end

  local probe = probes[1]
  t.eq(probe.current.state, fixture.current_state, fixture.name .. ": probe current state")
  t.eq(probe.current.version, fixture.current_version, fixture.name .. ": probe current version")
  t.eq(probe.from_states[1], "unmanaged", fixture.name .. ": probe source state")
  t.eq(#probe.from_states, 1, fixture.name .. ": probe source state count")
  t.eq(probe.to_state, "thinking", fixture.name .. ": probe target state")
  t.eq(probe.incoming_version, fixture.incoming_version, fixture.name .. ": probe incoming version")
  t.eq(probe.target_version, nil, fixture.name .. ": probe target version")

  local definition = catalog.definition(POLICY_ID)
  local variant = definition and definition.variants and definition.variants[VARIANT]
  t.is_true(variant ~= nil, fixture.name .. ": catalog variant exists")
  t.eq(#variant.source_states, #probe.from_states, fixture.name .. ": catalog source state count matches probe")
  for index, source_state in ipairs(probe.from_states) do
    t.eq(variant.source_states[index], source_state, fixture.name .. ": catalog source state matches probe")
  end
  t.eq(variant.target_state, probe.to_state, fixture.name .. ": catalog target state matches probe")
  t.eq(variant.target_version, probe.target_version, fixture.name .. ": catalog target version matches probe")

  t.eq(#decisions, 1, fixture.name .. ": structured CAS decision count")
  local decision = decisions[1]
  t.eq(decision.dept, "observe_issue", fixture.name .. ": CAS decision department")
  t.eq(decision.from_state, "unmanaged", fixture.name .. ": logged source state")
  t.eq(decision.to_state, "thinking", fixture.name .. ": logged target state")
  t.is_true(type(decision.outcome) == "string", fixture.name .. ": legacy log outcome captured")
  t.is_true(type(decision.reason) == "string", fixture.name .. ": legacy log reason captured")

  t.eq(#boundary_calls, probe.outcome == "apply" and 1 or 0, fixture.name .. ": admission boundary reach")
  if boundary_reached then
    local boundary = boundary_calls[1]
    t.eq(boundary.dept, "observe_issue", fixture.name .. ": claim boundary department")
    t.eq(boundary.repo, event.repo, fixture.name .. ": claim boundary repo")
    t.eq(boundary.issue_number, event.number, fixture.name .. ": claim boundary issue")
    t.eq(boundary.proposal_id, "github-devloop/issue/owner/repo/42", fixture.name .. ": claim boundary proposal")
    t.eq(boundary.outcome, true, fixture.name .. ": claim boundary outcome")
  end

  local observed = observed_admission(probe, boundary_reached)
  t.eq(result.exit_code, fixture.expected_exit_code or 0, fixture.name .. ": department exit code")
  if fixture.effect_count ~= nil then
    t.eq(#result.raises, fixture.effect_count, fixture.name .. ": captured effect count")
  end

  local disposition = post_admission_disposition(result, decision, boundary_reached)
  t.eq(disposition, fixture.post_admission_disposition or "not-admitted", fixture.name .. ": post-admission disposition")
  if fixture.legacy_log_outcome ~= nil then
    t.eq(decision.outcome, fixture.legacy_log_outcome, fixture.name .. ": legacy log outcome")
  end

  local actual = catalog.resolve(POLICY_ID, evidence_from_probe(probe))
  t.eq(actual.status, observed.status, fixture.name .. ": admission status parity")
  t.eq(actual.reason_code, observed.reason_code, fixture.name .. ": admission reason parity")
  return "cas"
end

local function assert_malformed_fails_closed_before_cas()
  local event = h.issue()
  event.updated_at = nil

  local result, probes, decisions, boundary_calls = observe_department(function()
    return run_real_department(event)
  end)

  t.eq(result.exit_code, 0, "observe-issue-entry-malformed: department rejects cleanly")
  t.eq(#probes, 0, "observe-issue-entry-malformed: production rejects before CAS")
  t.eq(#boundary_calls, 0, "observe-issue-entry-malformed: admission boundary is not reached")
  t.eq(#decisions, 1, "observe-issue-entry-malformed: rejection decision count")

end

return {
  test_observe_issue_entry_unmanaged_source_is_admitted_and_emits = function()
    assert_catalog_matches_observed_decision({
      name = "observe-issue-entry-source-apply",
      current_state = nil,
      current_version = nil,
      incoming_version = V_EQUAL,
      effect_count = 3,
      post_admission_disposition = "effect-emitted(thinking)",
      legacy_log_outcome = "applied",
    })
  end,

  test_observe_issue_entry_target_state_is_pre_cas = function()
    assert_catalog_matches_observed_decision({
      name = "observe-issue-entry-target-idempotent",
      current_state = "thinking",
      current_version = V_EQUAL,
      incoming_version = V_EQUAL,
    })
  end,

  test_observe_issue_entry_older_event_is_pre_cas = function()
    assert_catalog_matches_observed_decision({
      name = "observe-issue-entry-older-stale",
      current_state = "thinking",
      current_version = V_EQUAL,
      incoming_version = V_OLDER,
    })
  end,

  test_observe_issue_entry_newer_event_at_target_is_pre_cas = function()
    assert_catalog_matches_observed_decision({
      name = "observe-issue-entry-newer-target-idempotent",
      current_state = "thinking",
      current_version = V_EQUAL,
      incoming_version = V_NEWER,
    })
  end,

  test_observe_issue_entry_ordering_equal_raw_different_is_pre_cas = function()
    t.is_true(
      V_ORDERING_EQUAL_CURRENT ~= V_ORDERING_EQUAL_INCOMING,
      "observe-issue-entry-ordering-equal-raw-different: fixture versions must be byte-different"
    )
    assert_catalog_matches_observed_decision({
      name = "observe-issue-entry-ordering-equal-raw-different",
      current_state = "thinking",
      current_version = V_ORDERING_EQUAL_CURRENT,
      incoming_version = V_ORDERING_EQUAL_INCOMING,
    })
  end,

  test_observe_issue_entry_malformed_payload_fails_closed_before_cas = function()
    assert_malformed_fails_closed_before_cas()
  end,
}
