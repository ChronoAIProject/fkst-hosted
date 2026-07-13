-- Non-circularity contract: legacy truth comes from the real loop department's
-- transition_status probe and structured CAS log. The shadow decision receives
-- only a separately sealed snapshot and never observes the legacy probe.

local devloop_logging = require("devloop.logging")
local devloop_state = require("devloop.state")
local h = require("tests.devloop_helpers")
local restart_authority = require("core.restart_authority")
local t = h.t
local core = h.core
local loop_department = require("departments.loop.main")

local OWNER = "github-devloop"
local SEMANTIC_VARIANT = "consensus-stalled"
local POLICY_ID = "cas.legacy_loop_plain_v1"
local EDGE_ID = "github-devloop/thinking/autonomous/consensus-stalled"
local V_CURRENT = "consensus:github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z"

local state_labels = {
  thinking = "fkst-dev:thinking",
  ready = "fkst-dev:ready",
  blocked = "fkst-dev:blocked",
}

local fixtures = {
  {
    name = "shadow-source-apply",
    current_state = "thinking",
    current_version = V_CURRENT,
    expected_exit_code = 0,
    needs_context = true,
  },
  {
    name = "shadow-target-idempotent",
    current_state = "blocked",
    current_version = V_CURRENT,
    expected_exit_code = 0,
  },
  {
    name = "shadow-older-pending",
    current_state = nil,
    current_version = nil,
    expected_exit_code = 1,
  },
  {
    -- Stale coverage is within the dedup_key == current.version regime only: this
    -- reaches cas_outcome "skip-advanced-or-diverged". The loop stale version-branch
    -- ("skip-stale(incoming version < current marker version)", when dedup_key is
    -- ordered-older) is a documented gap not yet modeled by the version-less shadow
    -- evidence -- see restart_authority.lua's header; close before grant-enablement.
    name = "shadow-newer-stale",
    current_state = "ready",
    current_version = V_CURRENT,
    expected_exit_code = 0,
  },
}

local function observe_department(run)
  local probes = {}
  local decisions = {}
  local original_transition = devloop_state.transition_status
  local original_log_cas = devloop_logging.log_cas_decision

  devloop_state.transition_status = function(
    current,
    from_states,
    to_state,
    incoming_version,
    target_version
  )
    local outcome = original_transition(
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
  devloop_state.transition_status = original_transition
  if not ok then
    error(result, 0)
  end
  return result, probes, decisions
end

local function run_real_department(payload)
  local raises = {}
  local original_raise = raise
  raise = function(queue, raised_payload)
    table.insert(raises, { queue = queue, payload = raised_payload })
  end
  local ok, failure = pcall(loop_department.pipeline, {
    queue = "consensus.consensus_converge",
    payload = payload,
    ts = "2026-06-03T01:02:03Z",
  })
  raise = original_raise
  return {
    exit_code = ok and 0 or 1,
    error = ok and nil or tostring(failure),
    raises = raises,
  }
end

local function fixture_comments(event, fixture)
  if fixture.current_state == nil then
    return {}
  end
  return {
    core.state_marker(
      event.proposal_id,
      fixture.current_state,
      fixture.current_version
    ),
  }
end

local function observed_admission(probe)
  if probe.outcome == "apply" then
    return { status = "apply", reason_code = "apply" }
  end
  if probe.outcome == "idempotent" then
    return { status = "idempotent", reason_code = "already-at-target" }
  end
  if probe.outcome == "pending" then
    return { status = "pending", reason_code = "source-marker-not-visible" }
  end
  if probe.outcome == "stale" then
    return { status = "stale", reason_code = "advanced-or-diverged" }
  end
  error("unexpected loop plain CAS probe outcome: " .. tostring(probe.outcome))
end

local function assert_bidirectional(actual, expected, field, context)
  t.eq(actual[field], expected[field], context .. ": shadow-to-legacy " .. field)
  t.eq(expected[field], actual[field], context .. ": legacy-to-shadow " .. field)
end

local function assert_case(fixture)
  local event = h.unresolved({
    dedup_key = V_CURRENT,
    round = 0,
    narrowed_question = "Which fact resolves the remaining gap?",
    angle_digests = {
      { angle = "minimal", verdict = "abstain", digest = "shadow-parity" },
    },
  })
  local labels = { "fkst-dev:enabled" }
  if fixture.current_state ~= nil then
    table.insert(labels, state_labels[fixture.current_state])
  end
  h.mock_issue_loop(labels, fixture_comments(event, fixture))
  if fixture.needs_context then
    h.mock_context_bundle(event)
  end

  local result, probes, decisions = observe_department(function()
    return run_real_department(event)
  end)

  t.eq(result.exit_code, fixture.expected_exit_code, fixture.name .. ": department exit code")
  t.eq(#probes, 1, fixture.name .. ": real department CAS probe count")
  t.eq(#decisions, 1, fixture.name .. ": structured CAS decision count")
  local probe = probes[1]
  local decision = decisions[1]
  t.eq(probe.current.state, fixture.current_state, fixture.name .. ": observed current state")
  t.eq(probe.current.version, fixture.current_version, fixture.name .. ": observed current version")
  t.eq(probe.from_states[1], "thinking", fixture.name .. ": observed source state")
  t.eq(#probe.from_states, 1, fixture.name .. ": observed source state count")
  t.eq(probe.to_state, "blocked", fixture.name .. ": observed target state")
  t.eq(probe.incoming_version, nil, fixture.name .. ": plain probe incoming version")
  t.eq(probe.target_version, nil, fixture.name .. ": plain probe target version")
  t.eq(decision.dept, "loop", fixture.name .. ": legacy decision department")

  local legacy = observed_admission(probe)
  legacy.cas_outcome = decision.outcome
  local sealed = restart_authority.seal_snapshot({
    owner = OWNER,
    proposal_id = event.proposal_id,
    current = {
      state = fixture.current_state,
      version = fixture.current_version,
    },
  })
  local shadow = restart_authority.decide_transition(sealed, {
    semantic_variant = SEMANTIC_VARIANT,
  })

  assert_bidirectional(shadow, legacy, "status", fixture.name)
  assert_bidirectional(shadow, legacy, "reason_code", fixture.name)
  assert_bidirectional(shadow, legacy, "cas_outcome", fixture.name)
  t.eq(shadow.edge_id, EDGE_ID, fixture.name .. ": selected edge id")
  t.eq(shadow.cas_policy_id, POLICY_ID, fixture.name .. ": selected CAS policy")
  t.eq(shadow.evidence.status, "complete", fixture.name .. ": evidence status")
  t.eq(#shadow.evidence.refs, 0, fixture.name .. ": default evidence refs")
  t.eq(shadow.evidence.facts.source, "thinking", fixture.name .. ": evidence source")
  t.eq(shadow.evidence.facts.target, "blocked", fixture.name .. ": evidence target")
  t.eq(shadow.grant, nil, fixture.name .. ": grant disabled")
end

local function assert_illegal(actual, reason_code, cas_outcome, context)
  t.eq(actual.status, "illegal", context .. ": status")
  t.eq(actual.reason_code, reason_code, context .. ": reason code")
  t.eq(actual.cas_outcome, cas_outcome, context .. ": CAS outcome")
  t.eq(actual.grant, nil, context .. ": grant disabled")
end

local function sealed_snapshot()
  return restart_authority.seal_snapshot({
    owner = OWNER,
    proposal_id = "github-devloop/issue/owner/repo/42",
    current = { state = "thinking", version = V_CURRENT },
  })
end

return {
  test_shadow_decider_matches_legacy_loop_plain_cas_triplets = function()
    for _, fixture in ipairs(fixtures) do
      assert_case(fixture)
    end
  end,

  test_shadow_decider_rejects_unsealed_snapshot = function()
    local actual = restart_authority.decide_transition({
      owner = OWNER,
      current = { state = "thinking", version = V_CURRENT },
    }, {
      semantic_variant = SEMANTIC_VARIANT,
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
    })
    assert_illegal(actual, "unknown-variant", "illegal(unknown-variant)", "unknown variant")
  end,

  test_shadow_decider_fences_other_thinking_to_blocked_edge = function()
    local actual = restart_authority.decide_transition(sealed_snapshot(), {
      semantic_variant = "issue_reconcile_true_stall",
    })
    assert_illegal(
      actual,
      "unsupported-shadow-edge",
      "illegal(unsupported-shadow-edge)",
      "unsupported shadow edge"
    )
  end,
}
