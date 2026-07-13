-- Non-circularity contract: legacy truth comes from the real review_result
-- department's cyclic CAS probe and structured CAS log. The shadow decision is
-- built independently from fixture and event facts and never observes that probe.

local catalog = require("devloop.restart_cas_catalog")
local devloop_base = require("devloop.base")
local devloop_logging = require("devloop.logging")
local devloop_state = require("devloop.state")
local m_builders = require("devloop.markers.builders")
local transition_version = require("contract.transition_version")
local h = require("tests.devloop_helpers")
local restart_authority = require("core.restart_authority")
local t = h.t
local core = h.core
local review_result_department = require("departments.review_result.main")

local OWNER = "github-devloop-pr"
local SEMANTIC_VARIANT = "approved"
local POLICY_ID = "cas.legacy_review_result_v1"
local EDGE_ID = "github-devloop-pr/reviewing/autonomous/approved"
local V_EQUAL = "2026-06-03T01-02-03Z"
local V_ORDERING_EQUAL_CURRENT = "v-loop-01"
local V_ORDERING_EQUAL_INCOMING = "v-loop-1"

local fixtures = {
  {
    name = "shadow-review-result-source-equal-apply",
    current_state = "reviewing",
    current_version = V_EQUAL,
    incoming_version = V_EQUAL,
    expected_exit_code = 0,
    expected_status = "apply",
  },
  {
    name = "shadow-review-result-target-idempotent",
    current_state = "merge-ready",
    current_version = V_EQUAL,
    incoming_version = V_EQUAL,
    expected_exit_code = 0,
    expected_status = "idempotent",
  },
  {
    name = "shadow-review-result-source-missing-pending",
    current_state = nil,
    current_version = nil,
    incoming_version = V_EQUAL,
    expected_exit_code = 1,
    expected_status = "pending",
  },
  {
    name = "shadow-review-result-safe-overlay-mismatch-stale",
    current_state = "reviewing",
    current_version = V_ORDERING_EQUAL_CURRENT,
    incoming_version = V_ORDERING_EQUAL_INCOMING,
    expected_exit_code = 0,
    expected_probe_outcome = "apply",
    expected_status = "stale",
  },
}

local function mock_branch_config()
  t.mock_command('printf %s "$FKST_DEVLOOP_UPSTREAM_BRANCH"', {
    stdout = "dev",
    stderr = "",
    exit_code = 0,
  })
  t.mock_command('printf %s "$FKST_DEVLOOP_INTEGRATION_BRANCH"', {
    stdout = "",
    stderr = "",
    exit_code = 0,
  })
end

local function observe_department(run)
  local probes = {}
  local decisions = {}
  local original_cyclic = devloop_state.cyclic_transition_status
  local original_log_cas = devloop_logging.log_cas_decision

  devloop_state.cyclic_transition_status = function(
    current,
    from_states,
    to_state,
    incoming_version,
    target_version
  )
    local outcome = original_cyclic(
      current,
      from_states,
      to_state,
      incoming_version,
      target_version
    )
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
  devloop_logging.log_cas_decision = function(
    dept,
    proposal_id,
    current,
    from_state,
    to_state,
    outcome,
    reason
  )
    table.insert(decisions, {
      dept = dept,
      proposal_id = proposal_id,
      current = current,
      from_state = from_state,
      to_state = to_state,
      outcome = outcome,
      reason = reason,
    })
    return original_log_cas(
      dept,
      proposal_id,
      current,
      from_state,
      to_state,
      outcome,
      reason
    )
  end

  local ok, result = pcall(run)
  devloop_logging.log_cas_decision = original_log_cas
  devloop_state.cyclic_transition_status = original_cyclic
  if not ok then
    error(result, 0)
  end
  return result, probes, decisions
end

local function observe_shadow(run)
  local evidence = nil
  local original_resolve = catalog.resolve
  catalog.resolve = function(policy_id, candidate, projection)
    evidence = candidate
    return original_resolve(policy_id, candidate, projection)
  end
  local ok, result = pcall(run)
  catalog.resolve = original_resolve
  if not ok then
    error(result, 0)
  end
  return result, evidence
end

local function review_event(fixture)
  local proposal_id = devloop_base.pr_review_proposal_id(
    "owner/repo",
    7,
    fixture.incoming_version,
    "def456"
  )
  return h.review_reached({
    proposal_id = proposal_id,
    dedup_key = "consensus:" .. proposal_id .. "/review",
    decision = "approve",
    body = "Review consensus approves the diff.",
  })
end

local function prepare_fixture(fixture)
  mock_branch_config()
  h.mock_default_issue_claim()
  local comments = {
    m_builders.pr_origin_marker(
      "github-devloop/issue/owner/repo/42",
      "42",
      "devloop-owner-repo-42-01HY",
      fixture.incoming_version,
      "dev"
    ),
  }
  if fixture.current_state ~= nil then
    table.insert(comments, core.state_marker(
      "github-devloop/issue/owner/repo/42",
      fixture.current_state,
      fixture.current_version
    ))
  end
  if fixture.current_state == nil then
    h.mock_pr_origin_for({
      comments = comments,
      head = "devloop-owner-repo-42-01HY",
      head_sha = "def456",
      state = "OPEN",
      base_branch = "dev",
    })
  else
    h.mock_pr_origin(comments, "devloop-owner-repo-42-01HY", "def456", "OPEN", "dev")
  end
  h.mock_pr_normal_risk_diff_name_only()
end

local function run_real_department(event)
  local raises = {}
  local original_raise = raise
  raise = function(queue, payload)
    table.insert(raises, { queue = queue, payload = payload })
  end
  local ok, failure = pcall(review_result_department.pipeline, {
    queue = "consensus.consensus_reached",
    payload = event,
  })
  raise = original_raise
  return {
    exit_code = ok and 0 or 1,
    error = ok and nil or tostring(failure),
    raises = raises,
  }
end

local function observed_admission(probe, decision)
  local outcome = decision.outcome
  if outcome == "applied" then
    return { status = "apply", reason_code = "apply", cas_outcome = outcome }
  end
  if outcome == "skip-idempotent(already at to_state)" then
    return {
      status = "idempotent",
      reason_code = "already-at-target",
      cas_outcome = outcome,
    }
  end
  if outcome == "retry-pending(from-state marker not yet visible)" then
    return {
      status = "pending",
      reason_code = "source-marker-not-visible",
      cas_outcome = outcome,
    }
  end
  if outcome == "skip-stale(version-mismatch)" then
    return { status = "stale", reason_code = "version-mismatch", cas_outcome = outcome }
  end
  if outcome == "skip-stale(incoming version < current marker version)" then
    return { status = "stale", reason_code = "incoming-version-older", cas_outcome = outcome }
  end
  if outcome == "skip-advanced-or-diverged" then
    return { status = "stale", reason_code = "advanced-or-diverged", cas_outcome = outcome }
  end
  error(
    "unexpected review_result CAS outcome after probe "
      .. tostring(probe.outcome)
      .. ": "
      .. tostring(outcome)
  )
end

local function assert_bidirectional(actual, expected, field, context)
  t.eq(actual[field], expected[field], context .. ": shadow-to-legacy " .. field)
  t.eq(expected[field], actual[field], context .. ": legacy-to-shadow " .. field)
end

local function assert_case(fixture)
  local event = review_event(fixture)
  prepare_fixture(fixture)
  local result, probes, decisions = observe_department(function()
    return run_real_department(event)
  end)

  t.eq(result.exit_code, fixture.expected_exit_code, fixture.name .. ": department exit code")
  t.eq(#probes, 1, fixture.name .. ": real department CAS probe count")
  t.eq(#decisions, 1, fixture.name .. ": structured CAS decision count")
  local probe = probes[1]
  local decision = decisions[1]
  local safe_current_version = transition_version.safe_version_segment(fixture.current_version or "")
  t.eq(probe.current.state, fixture.current_state, fixture.name .. ": observed current state")
  t.eq(probe.current.version, safe_current_version, fixture.name .. ": observed safe current version")
  t.eq(probe.from_states[1], "reviewing", fixture.name .. ": observed source state")
  t.eq(#probe.from_states, 1, fixture.name .. ": observed source state count")
  t.eq(probe.to_state, "merge-ready", fixture.name .. ": observed target state")
  t.eq(probe.incoming_version, fixture.incoming_version, fixture.name .. ": observed incoming version")
  t.eq(probe.target_version, nil, fixture.name .. ": observed target version")
  if fixture.expected_probe_outcome ~= nil then
    t.eq(probe.outcome, fixture.expected_probe_outcome, fixture.name .. ": literal probe outcome")
  end
  t.eq(decision.dept, "review_result", fixture.name .. ": legacy decision department")
  t.eq(decision.from_state, "reviewing", fixture.name .. ": legacy decision source")
  t.eq(decision.to_state, "merge-ready", fixture.name .. ": legacy decision target")

  local legacy = observed_admission(probe, decision)
  local sealed = restart_authority.seal_snapshot({
    owner = OWNER,
    proposal_id = "github-devloop/issue/owner/repo/42",
    current = {
      state = fixture.current_state,
      version = fixture.current_version,
    },
  })
  local shadow, evidence = observe_shadow(function()
    return restart_authority.decide_transition(sealed, {
      semantic_variant = SEMANTIC_VARIANT,
      incoming_version = fixture.incoming_version,
      target_version = fixture.target_version,
      overlay_version = fixture.incoming_version,
    })
  end)

  assert_bidirectional(shadow, legacy, "status", fixture.name)
  t.eq(shadow.status, fixture.expected_status, fixture.name .. ": expected shadow status")
  assert_bidirectional(shadow, legacy, "reason_code", fixture.name)
  assert_bidirectional(shadow, legacy, "cas_outcome", fixture.name)
  t.eq(shadow.edge_id, EDGE_ID, fixture.name .. ": selected edge id")
  t.eq(shadow.cas_policy_id, POLICY_ID, fixture.name .. ": selected CAS policy")
  t.eq(shadow.grant, nil, fixture.name .. ": grant disabled")
  t.eq(evidence.current.state, fixture.current_state, fixture.name .. ": evidence current state")
  t.eq(evidence.current.version, safe_current_version, fixture.name .. ": evidence safe current version")
  t.eq(evidence.variant, "reviewing_to_merge_ready", fixture.name .. ": evidence variant")
  t.eq(evidence.incoming_version, fixture.incoming_version, fixture.name .. ": evidence incoming version")
  t.eq(evidence.target_version, fixture.target_version, fixture.name .. ": evidence target version")
  t.eq(evidence.overlay_version, fixture.incoming_version, fixture.name .. ": evidence overlay version")
end

local function sealed_snapshot()
  return restart_authority.seal_snapshot({
    owner = OWNER,
    proposal_id = "github-devloop/issue/owner/repo/42",
    current = { state = "reviewing", version = V_EQUAL },
  })
end

local function assert_illegal(actual, reason_code, cas_outcome, context)
  t.eq(actual.status, "illegal", context .. ": status")
  t.eq(actual.reason_code, reason_code, context .. ": reason code")
  t.eq(actual.cas_outcome, cas_outcome, context .. ": CAS outcome")
  t.eq(actual.grant, nil, context .. ": grant disabled")
end

return {
  test_shadow_decider_matches_legacy_review_result_cyclic_cas_triplets = function()
    t.is_true(
      transition_version.safe_version_segment(V_ORDERING_EQUAL_CURRENT)
        ~= transition_version.safe_version_segment(V_ORDERING_EQUAL_INCOMING),
      "safe-overlay fixture safe versions must be byte-different"
    )
    t.eq(
      transition_version.compare(
        transition_version.safe_version_segment(V_ORDERING_EQUAL_CURRENT),
        transition_version.safe_version_segment(V_ORDERING_EQUAL_INCOMING)
      ),
      0,
      "safe-overlay fixture versions must be ordering-equal"
    )
    for _, fixture in ipairs(fixtures) do
      assert_case(fixture)
    end
  end,

  test_shadow_decider_rejects_unsealed_snapshot = function()
    local actual = restart_authority.decide_transition({
      owner = OWNER,
      current = { state = "reviewing", version = V_EQUAL },
    }, {
      semantic_variant = SEMANTIC_VARIANT,
      incoming_version = V_EQUAL,
      overlay_version = V_EQUAL,
    })
    assert_illegal(
      actual,
      "unsealed-or-foreign-snapshot",
      "illegal(unsealed)",
      "unsealed snapshot"
    )
  end,

  test_shadow_decider_rejects_unknown_semantic_variant = function()
    local actual = restart_authority.decide_transition(sealed_snapshot(), {
      semantic_variant = "unknown-shadow-variant",
      incoming_version = V_EQUAL,
      overlay_version = V_EQUAL,
    })
    assert_illegal(actual, "unknown-variant", "illegal(unknown-variant)", "unknown variant")
  end,

  test_shadow_decider_requires_cyclic_incoming_version = function()
    local actual = restart_authority.decide_transition(sealed_snapshot(), {
      semantic_variant = SEMANTIC_VARIANT,
      overlay_version = V_EQUAL,
    })
    assert_illegal(
      actual,
      "incoming-version-required",
      "illegal(incoming-version-required)",
      "missing cyclic incoming version"
    )
  end,
}
