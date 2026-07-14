-- Non-circularity contract: production truth comes from the real fix
-- department's CAS probe and the review-reject fact admission boundary. Effects
-- and legacy CAS logs are recorded as separate post-admission observations. This
-- test never computes the expected result with a devloop.state transition helper.

local catalog = require("devloop.restart_cas_catalog")
local owner_pending_projection = require("devloop.restart_owner_pending_projection")
local inventories = {
  canonicalization = require("core.restart.canonicalization_inventory"),
  entry = require("core.restart.entry_inventory"),
  operator_reentry = require("core.restart.operator_reentry_inventory"),
}
local devloop_base = require("devloop.base")
local devloop_logging = require("devloop.logging")
local m_facts = require("devloop.markers.facts")
local payloads_builders = require("devloop.payloads.builders")
local devloop_state = require("devloop.state")
local dispatch_live_run = require("devloop.dispatch_live_run")
local h = require("tests.devloop_helpers")
local restart_authority = require("core.restart_authority")
local t = h.t
local core = h.core
local projection = owner_pending_projection.derive(core.restart_package_name, core.restart_transition_table(), inventories)
local fix_department = require("departments.fix.main")

local POLICY_ID = "cas.legacy_fix_v1"
local VARIANT = "fixing_to_reviewing"
local OWNER = core.restart_package_name
local V_OLDER = "ready/consensus-github-devloop/issue/owner/repo/42/2026-06-02T01-02-03Z"
local V_EQUAL = "ready/consensus-github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z"
local V_NEWER = "ready/consensus-github-devloop/issue/owner/repo/42/2026-06-04T01-02-03Z"
local V_ORDERING_EQUAL_CURRENT = V_EQUAL .. "/loop/01"
local V_ORDERING_EQUAL_INCOMING = V_EQUAL .. "/loop/1"

local function fixing_event(version)
  local base = h.fixing()
  return payloads_builders.build_devloop_fixing_payload({
    proposal_id = base.proposal_id,
    impl_version = version,
  }, base.pr_number, {
    review_proposal_id = base.review_proposal_id,
    review_dedup_key = base.review_dedup_key,
    reviewed_head_sha = base.reviewed_head_sha,
    blocking_gap = base.blocking_gap,
  }, base.source_ref)
end

local function observe_department(run)
  local probes = {}
  local decisions = {}
  local boundary_calls = {}
  local original_cyclic = devloop_state.cyclic_transition_status
  local original_log_cas = devloop_logging.log_cas_decision
  local original_review_reject_fact = m_facts.review_reject_fact
  -- A fresh fixture has no live codex run. Force that ground truth so a future
  -- post-admission path cannot inherit a cross-case live-run registry entry.
  local original_dispatch_live_run_dedup = dispatch_live_run.dispatch_live_run_dedup
  dispatch_live_run.dispatch_live_run_dedup = function()
    return false
  end

  devloop_state.cyclic_transition_status = function(current, from_states, to_state, incoming_version, target_version)
    local outcome = original_cyclic(current, from_states, to_state, incoming_version, target_version)
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
  m_facts.review_reject_fact = function(comments, proposal_id, version)
    table.insert(boundary_calls, {
      comments = comments,
      proposal_id = proposal_id,
      version = version,
    })
    return original_review_reject_fact(comments, proposal_id, version)
  end

  local ok, result = pcall(run)
  dispatch_live_run.dispatch_live_run_dedup = original_dispatch_live_run_dedup
  m_facts.review_reject_fact = original_review_reject_fact
  devloop_logging.log_cas_decision = original_log_cas
  devloop_state.cyclic_transition_status = original_cyclic
  if not ok then
    error(result, 0)
  end
  return result, probes, decisions, boundary_calls
end

local function current_fact(state, version)
  return { state = state, version = version }
end

local function evidence_from_fixture(fixture)
  return {
    current = current_fact(fixture.current_state, fixture.current_version),
    variant = VARIANT,
    incoming_version = fixture.incoming_version,
    target_version = core.next_fix_version(fixture.incoming_version),
    overlay_version = fixture.incoming_version,
  }
end

local function observe_shadow(run)
  local evidence = nil
  local original_resolve = catalog.resolve
  catalog.resolve = function(policy_id, candidate, candidate_projection)
    evidence = candidate
    return original_resolve(policy_id, candidate, candidate_projection)
  end
  local ok, result = pcall(run)
  catalog.resolve = original_resolve
  if not ok then
    error(result, 0)
  end
  return result, evidence
end

local function observed_admission(probe, decision, boundary_reached)
  if probe.outcome == "pending" then
    return { status = "pending", reason_code = "source-marker-not-visible", cas_outcome = decision.outcome }
  end
  if probe.outcome == "idempotent" then
    return { status = "idempotent", reason_code = "already-at-target", cas_outcome = decision.outcome }
  end
  if probe.outcome == "stale" then
    if tostring(probe.incoming_version or "") ~= tostring(probe.current.version or "") then
      return { status = "stale", reason_code = "incoming-version-older", cas_outcome = decision.outcome }
    end
    return { status = "stale", reason_code = "advanced-or-diverged", cas_outcome = decision.outcome }
  end
  if probe.outcome ~= "apply" then
    error("fix admission probe returned an unknown outcome: " .. tostring(probe.outcome))
  end

  local legacy_outcome = tostring(decision and decision.outcome or "")
  local legacy_reason = tostring(decision and decision.reason or "")
  if not boundary_reached and legacy_reason:find("not currently fixing", 1, true) ~= nil then
    return { status = "stale", reason_code = "from-state-mismatch", cas_outcome = legacy_outcome }
  end
  if not boundary_reached and legacy_outcome:find("version-mismatch", 1, true) ~= nil then
    return { status = "stale", reason_code = "version-mismatch", cas_outcome = legacy_outcome }
  end
  if boundary_reached then
    return { status = "apply", reason_code = "apply", cas_outcome = "applied" }
  end
  error("fix admission apply did not reach a classified guard")
end

local function post_admission_disposition(result, decision, boundary_reached)
  if not boundary_reached then
    return "not-admitted"
  end
  if #(result.raises or {}) > 0 then
    return "effect-emitted"
  end
  local outcome = tostring(decision and decision.outcome or "")
  if outcome:find("fix feedback marker not visible", 1, true) ~= nil then
    return "feedback-pending"
  end
  if outcome:find("live-exec-ref", 1, true) ~= nil then
    return "liveness-deferred"
  end
  return "post-admission-no-effect"
end

local function run_real_department(event)
  local raises = {}
  local original_raise = raise
  raise = function(queue, payload)
    table.insert(raises, { queue = queue, payload = payload })
  end
  local ok, failure = pcall(fix_department.pipeline, {
    queue = "devloop_fixing",
    payload = event,
  })
  raise = original_raise
  return {
    exit_code = ok and 0 or 1,
    error = ok and nil or tostring(failure),
    raises = raises,
  }
end

local function mock_current_pr(event, fixture)
  local comments = {}
  if fixture.current_state ~= nil then
    table.insert(comments, core.state_marker(event.proposal_id, fixture.current_state, fixture.current_version))
  end
  h.mock_bot_env()
  h.mock_default_issue_claim()
  local branch = devloop_base.implement_branch("owner/repo", "42", event.version)
  h.mock_pr_fix(comments, branch, event.reviewed_head_sha)
end

local function assert_catalog_matches_observed_decision(fixture)
  local event = fixing_event(fixture.incoming_version)
  mock_current_pr(event, fixture)

  local result, probes, decisions, boundary_calls = observe_department(function()
    return run_real_department(event)
  end)

  t.eq(#probes, 1, fixture.name .. ": real department CAS probe count")
  local probe = probes[1]
  t.eq(probe.current.state, fixture.current_state, fixture.name .. ": probe current state")
  t.eq(probe.current.version, fixture.current_version, fixture.name .. ": probe current version")
  t.eq(probe.from_states[1], "fixing", fixture.name .. ": probe source state")
  t.eq(#probe.from_states, 1, fixture.name .. ": probe source state count")
  t.eq(probe.to_state, "reviewing", fixture.name .. ": probe target state")
  t.eq(probe.incoming_version, fixture.incoming_version, fixture.name .. ": probe incoming version")
  t.eq(probe.target_version, core.next_fix_version(fixture.incoming_version), fixture.name .. ": probe target version")

  t.eq(#decisions, 1, fixture.name .. ": structured CAS decision count")
  local decision = decisions[1]
  t.eq(decision.dept, "fix", fixture.name .. ": CAS decision department")
  t.eq(decision.from_state, "fixing", fixture.name .. ": logged source state")
  t.is_true(type(decision.outcome) == "string", fixture.name .. ": legacy log outcome captured")
  t.is_true(type(decision.reason) == "string", fixture.name .. ": legacy log reason captured")

  local boundary_reached = #boundary_calls > 0
  t.eq(#boundary_calls, fixture.boundary_reached and 1 or 0, fixture.name .. ": admission boundary reach")
  if boundary_reached then
    t.eq(boundary_calls[1].proposal_id, event.proposal_id, fixture.name .. ": boundary proposal")
    t.eq(boundary_calls[1].version, event.version, fixture.name .. ": boundary version")
  end

  local observed = observed_admission(probe, decision, boundary_reached)
  local actual = catalog.resolve(POLICY_ID, evidence_from_fixture(fixture), projection)
  t.eq(actual.status, observed.status, fixture.name .. ": admission status parity")
  t.eq(actual.reason_code, observed.reason_code, fixture.name .. ": admission reason parity")
  if fixture.probe_outcome ~= nil then
    t.eq(probe.outcome, fixture.probe_outcome, fixture.name .. ": literal probe outcome")
  end
  if fixture.admission_status ~= nil then
    t.eq(observed.status, fixture.admission_status, fixture.name .. ": observed admission status")
    t.eq(actual.status, fixture.admission_status, fixture.name .. ": catalog admission status")
  end
  if fixture.admission_reason_code ~= nil then
    t.eq(observed.reason_code, fixture.admission_reason_code, fixture.name .. ": observed admission reason")
    t.eq(actual.reason_code, fixture.admission_reason_code, fixture.name .. ": catalog admission reason")
  end
  t.eq(result.exit_code, fixture.expected_exit_code or 0, fixture.name .. ": department exit code")
  t.eq(#result.raises, fixture.effect_count or 0, fixture.name .. ": captured effect count")

  local disposition = post_admission_disposition(result, decision, boundary_reached)
  t.eq(disposition, fixture.post_admission_disposition or "not-admitted", fixture.name .. ": post-admission disposition")
  if fixture.legacy_log_outcome ~= nil then
    t.eq(decision.outcome, fixture.legacy_log_outcome, fixture.name .. ": legacy log outcome")
  end
  return {
    probe = probe,
    decision = decision,
    observed = observed,
  }
end

local function assert_bidirectional(actual, expected, field, context)
  t.eq(actual[field], expected[field], context .. ": shadow-to-production " .. field)
  t.eq(expected[field], actual[field], context .. ": production-to-shadow " .. field)
end

local function assert_fix_shadow_case(fixture)
  local production = assert_catalog_matches_observed_decision(fixture)
  local sealed_snapshot = restart_authority.seal_snapshot({
    owner = OWNER,
    current = {
      state = fixture.current_state,
      version = fixture.current_version,
    },
  })
  local incoming_version = fixture.incoming_version
  local overlay_version = fixture.incoming_version
  local target_version = core.next_fix_version(fixture.incoming_version)
  local intent = {
    semantic_variant = "revision_published",
    target = "reviewing",
    incoming_version = incoming_version,
    target_version = target_version,
    overlay_version = overlay_version,
  }
  t.eq(intent.source_boundary, nil, fixture.name .. ": nil boundary is omitted from shadow intent")

  local shadow, evidence = observe_shadow(function()
    return restart_authority.decide_transition(sealed_snapshot, intent)
  end)

  assert_bidirectional(shadow, production.observed, "status", fixture.name)
  assert_bidirectional(shadow, production.observed, "reason_code", fixture.name)
  assert_bidirectional(shadow, production.observed, "cas_outcome", fixture.name)
  t.eq(shadow.edge_id, "github-devloop-pr/fixing/autonomous/revision_published", fixture.name .. ": selected edge")
  t.eq(shadow.cas_policy_id, POLICY_ID, fixture.name .. ": selected CAS policy")
  t.eq(shadow.grant, nil, fixture.name .. ": grant disabled")
  t.eq(evidence.current.state, fixture.current_state, fixture.name .. ": evidence current state")
  t.eq(evidence.current.version, fixture.current_version or "", fixture.name .. ": evidence raw current version")
  t.eq(evidence.variant, VARIANT, fixture.name .. ": evidence variant")
  t.eq(evidence.incoming_version, incoming_version, fixture.name .. ": independently derived incoming version")
  t.eq(evidence.target_version, target_version, fixture.name .. ": independently derived target version")
  t.eq(evidence.overlay_version, overlay_version, fixture.name .. ": independently derived overlay version")
end

local function assert_rejected_before_cas(name, payload, expected_reason_code)
  local result, probes, _, boundary_calls = observe_department(function()
    return run_real_department(payload)
  end)
  t.eq(result.exit_code, 0, name .. ": unsupported payload is rejected without a pipeline error")
  t.eq(#probes, 0, name .. ": invalid production input must not reach CAS")
  t.eq(#boundary_calls, 0, name .. ": invalid production input must not reach the admission boundary")
  local evidence = {
    current = current_fact("fixing", V_EQUAL),
    variant = VARIANT,
    incoming_version = payload.version,
    overlay_version = payload.version,
  }
  t.eq(evidence.incoming_version, payload.version, name .. ": catalog incoming version comes from rejected payload")
  t.eq(evidence.overlay_version, payload.version, name .. ": catalog overlay version comes from rejected payload")
  local resolved = catalog.resolve(POLICY_ID, evidence, projection)
  t.eq(resolved.status, "illegal", name .. ": catalog status")
  t.eq(resolved.reason_code, expected_reason_code, name .. ": catalog reason")
  t.eq(resolved.cas_outcome, "illegal(" .. expected_reason_code .. ")", name .. ": catalog fails closed")
end

return {
  test_shadow_fixing_to_reviewing_apply = function()
    assert_fix_shadow_case({
      name = "shadow-fix-apply",
      current_state = "fixing",
      current_version = V_EQUAL,
      incoming_version = V_EQUAL,
      boundary_reached = true,
      expected_exit_code = 1,
      post_admission_disposition = "feedback-pending",
      legacy_log_outcome = "retry-pending(fix feedback marker not visible)",
    })
  end,

  test_shadow_fixing_to_reviewing_idempotent = function()
    assert_fix_shadow_case({
      name = "shadow-fix-idempotent",
      current_state = "reviewing",
      current_version = V_EQUAL,
      incoming_version = V_EQUAL,
    })
  end,

  test_shadow_fixing_to_reviewing_pending = function()
    assert_fix_shadow_case({
      name = "shadow-fix-pending",
      current_state = nil,
      current_version = nil,
      incoming_version = V_EQUAL,
      expected_exit_code = 1,
    })
  end,

  test_shadow_fixing_to_reviewing_raw_overlay_stale = function()
    assert_fix_shadow_case({
      name = "shadow-fix-raw-overlay-stale",
      current_state = "fixing",
      current_version = V_ORDERING_EQUAL_CURRENT,
      incoming_version = V_ORDERING_EQUAL_INCOMING,
      probe_outcome = "apply",
      admission_status = "stale",
      admission_reason_code = "version-mismatch",
      legacy_log_outcome = "skip-stale(version-mismatch)",
    })
  end,

  test_fix_source_equal_is_admitted_before_feedback_guard = function()
    assert_catalog_matches_observed_decision({
      name = "fix-source-equal",
      current_state = "fixing",
      current_version = V_EQUAL,
      incoming_version = V_EQUAL,
      boundary_reached = true,
      expected_exit_code = 1,
      post_admission_disposition = "feedback-pending",
      legacy_log_outcome = "retry-pending(fix feedback marker not visible)",
    })
  end,

  test_fix_target_state_is_idempotent_through_named_probe = function()
    assert_catalog_matches_observed_decision({
      name = "fix-target-idempotent",
      current_state = "reviewing",
      current_version = V_EQUAL,
      incoming_version = V_EQUAL,
    })
  end,

  test_fix_source_older_is_stale = function()
    assert_catalog_matches_observed_decision({
      name = "fix-source-older",
      current_state = "fixing",
      current_version = V_EQUAL,
      incoming_version = V_OLDER,
    })
  end,

  test_fix_source_newer_is_pending = function()
    assert_catalog_matches_observed_decision({
      name = "fix-source-newer",
      current_state = "fixing",
      current_version = V_EQUAL,
      incoming_version = V_NEWER,
      expected_exit_code = 1,
    })
  end,

  test_fix_missing_current_marker_is_pending = function()
    assert_catalog_matches_observed_decision({
      name = "fix-current-missing",
      current_state = nil,
      current_version = nil,
      incoming_version = V_EQUAL,
      expected_exit_code = 1,
    })
  end,

  test_fix_unrelated_state_is_stale = function()
    assert_catalog_matches_observed_decision({
      name = "fix-unrelated-stale",
      current_state = "blocked",
      current_version = V_EQUAL,
      incoming_version = V_EQUAL,
    })
  end,

  test_fix_predecessor_equal_is_stale_from_state_mismatch = function()
    assert_catalog_matches_observed_decision({
      name = "fix-predecessor-equal",
      current_state = "pr-open",
      current_version = V_EQUAL,
      incoming_version = V_EQUAL,
      probe_outcome = "apply",
      admission_status = "stale",
      admission_reason_code = "from-state-mismatch",
      legacy_log_outcome = "applied",
    })
  end,

  test_fix_ordering_equal_raw_different_is_stale_version_mismatch = function()
    t.is_true(
      V_ORDERING_EQUAL_CURRENT ~= V_ORDERING_EQUAL_INCOMING,
      "fix-ordering-equal-raw-different: fixture versions must be byte-different"
    )
    assert_catalog_matches_observed_decision({
      name = "fix-ordering-equal-raw-different",
      current_state = "fixing",
      current_version = V_ORDERING_EQUAL_CURRENT,
      incoming_version = V_ORDERING_EQUAL_INCOMING,
      probe_outcome = "apply",
      admission_status = "stale",
      admission_reason_code = "version-mismatch",
      legacy_log_outcome = "skip-stale(version-mismatch)",
    })
  end,

  test_fix_malformed_evidence_and_payload_fail_closed_before_cas = function()
    local payload = fixing_event(V_EQUAL)
    payload.version = 42
    assert_rejected_before_cas("fix-malformed-version", payload, "invalid-evidence")
  end,
}
