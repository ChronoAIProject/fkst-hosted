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
local consensus_result_department = require("departments.consensus_result.main")

local OWNER = "github-devloop"
local SEMANTIC_VARIANT = "consensus-stalled"
local POLICY_ID = "cas.legacy_loop_plain_v1"
local EDGE_ID = "github-devloop/thinking/autonomous/consensus-stalled"
local APPLY_ENTITLEMENT_ID = EDGE_ID .. "/apply"
local IDEMPOTENT_ENTITLEMENT_ID = EDGE_ID .. "/idempotent"
local V_CURRENT = "consensus:github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z"
local CONSENSUS_REACHED_VARIANT = "consensus-reached"
local CONSENSUS_RESULT_POLICY_ID = "cas.legacy_consensus_result_v1"
local CONSENSUS_REACHED_EDGE_ID = "github-devloop/thinking/autonomous/consensus-reached"
local V_OLDER = "consensus:github-devloop/issue/owner/repo/42/2026-06-02T01-02-03Z"
local V_NEWER = "consensus:github-devloop/issue/owner/repo/42/2026-06-04T01-02-03Z"
local V_ORDERING_EQUAL_CURRENT = V_CURRENT .. "/loop/01"
local V_ORDERING_EQUAL_INCOMING = V_CURRENT .. "/loop/1"

local state_labels = {
  thinking = "fkst-dev:thinking",
  ready = "fkst-dev:ready",
  blocked = "fkst-dev:blocked",
}

local fixtures = {
  {
    name = "shadow-source-apply-next-round",
    current_state = "thinking",
    current_version = V_CURRENT,
    expected_exit_code = 0,
    needs_context = true,
    expected_status = "apply",
    expected_entitlement_id = APPLY_ENTITLEMENT_ID,
    expected_effect_ids = {
      "consensus.proposal",
      "github-proxy.github_issue_comment_request",
    },
  },
  {
    name = "shadow-source-apply-terminal",
    current_state = "thinking",
    current_version = V_CURRENT,
    expected_exit_code = 0,
    essence_stall = true,
    expected_status = "apply",
    expected_entitlement_id = APPLY_ENTITLEMENT_ID,
    expected_effect_ids = {
      "github-proxy.github_issue_comment_request",
    },
  },
  {
    name = "shadow-target-idempotent",
    current_state = "blocked",
    current_version = V_CURRENT,
    expected_exit_code = 0,
    expected_status = "idempotent",
    expected_entitlement_id = IDEMPOTENT_ENTITLEMENT_ID,
    expected_effect_ids = {},
  },
  {
    name = "shadow-older-pending",
    current_state = nil,
    current_version = nil,
    expected_exit_code = 1,
    expected_status = "pending",
    expected_effect_ids = {},
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
    expected_status = "stale",
    expected_effect_ids = {},
  },
}

local consensus_result_fixtures = {
  {
    name = "shadow-consensus-result-source-equal-apply",
    current_state = "thinking",
    current_version = V_CURRENT,
    incoming_version = V_CURRENT,
    expected_exit_code = 0,
    expected_status = "apply",
  },
  {
    name = "shadow-consensus-result-target-ordering-equal-idempotent",
    current_state = "ready",
    current_version = V_ORDERING_EQUAL_CURRENT,
    incoming_version = V_ORDERING_EQUAL_INCOMING,
    expected_exit_code = 0,
    expected_status = "idempotent",
  },
  {
    name = "shadow-consensus-result-source-marker-missing-pending",
    current_state = nil,
    current_version = nil,
    incoming_version = V_NEWER,
    expected_exit_code = 1,
    expected_status = "pending",
  },
  {
    name = "shadow-consensus-result-incoming-older-stale",
    current_state = "thinking",
    current_version = V_CURRENT,
    incoming_version = V_OLDER,
    expected_exit_code = 0,
    expected_status = "stale",
  },
}

local function observe_department(run)
  local probes = {}
  local decisions = {}
  local apply_plans = {}
  local original_transition = devloop_state.transition_status
  local original_log_cas = devloop_logging.log_cas_decision
  local original_log_apply = devloop_logging.log_apply

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
  devloop_logging.log_apply = function(
    dept,
    proposal_id,
    to_state,
    version,
    labels,
    effect_ids
  )
    local copied_effect_ids = {}
    for _, effect_id in ipairs(effect_ids or {}) do
      table.insert(copied_effect_ids, effect_id)
    end
    table.insert(apply_plans, {
      dept = dept,
      proposal_id = proposal_id,
      to_state = to_state,
      version = version,
      labels = labels,
      effect_ids = copied_effect_ids,
    })
    return original_log_apply(
      dept,
      proposal_id,
      to_state,
      version,
      labels,
      effect_ids
    )
  end

  local ok, result = pcall(run)
  devloop_logging.log_apply = original_log_apply
  devloop_logging.log_cas_decision = original_log_cas
  devloop_state.transition_status = original_transition
  if not ok then
    error(result, 0)
  end
  return result, probes, decisions, apply_plans
end

local function observe_consensus_result_department(run)
  local probes = {}
  local decisions = {}
  local original_versioned = devloop_state.versioned_transition_status
  local original_log_cas = devloop_logging.log_cas_decision

  devloop_state.versioned_transition_status = function(current, from_states, to_state, incoming_version)
    local outcome = original_versioned(current, from_states, to_state, incoming_version)
    table.insert(probes, {
      current = current,
      from_states = from_states,
      to_state = to_state,
      incoming_version = incoming_version,
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
  devloop_state.versioned_transition_status = original_versioned
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

local function observed_consensus_result_admission(probe, decisions)
  local observed = nil
  if probe.outcome == "apply" then
    observed = { status = "apply", reason_code = "apply" }
  elseif probe.outcome == "idempotent" then
    observed = { status = "idempotent", reason_code = "already-at-target" }
  elseif probe.outcome == "pending" then
    observed = { status = "pending", reason_code = "source-marker-not-visible" }
  elseif probe.outcome == "stale" then
    local incoming_older = false
    for _, decision in ipairs(decisions) do
      if tostring(decision.outcome or ""):find(
        "incoming version < current marker version",
        1,
        true
      ) ~= nil then
        incoming_older = true
      end
    end
    observed = {
      status = "stale",
      reason_code = incoming_older and "incoming-version-older" or "advanced-or-diverged",
    }
  else
    error("unexpected consensus_result CAS probe outcome: " .. tostring(probe.outcome))
  end

  t.is_true(#decisions > 0, "consensus_result structured CAS decision captured")
  observed.cas_outcome = decisions[1].outcome
  for _, decision in ipairs(decisions) do
    t.eq(decision.outcome, observed.cas_outcome, "consensus_result CAS log outcomes agree")
  end
  return observed
end

local function assert_bidirectional(actual, expected, field, context)
  t.eq(actual[field], expected[field], context .. ": shadow-to-legacy " .. field)
  t.eq(expected[field], actual[field], context .. ": legacy-to-shadow " .. field)
end

local function assert_array(actual, expected, context)
  t.eq(type(actual), "table", context .. ": array type")
  t.eq(#actual, #expected, context .. ": array length")
  for index, value in ipairs(expected) do
    t.eq(actual[index], value, context .. ": array item " .. tostring(index))
  end
  for key in pairs(actual) do
    t.eq(
      type(key) == "number" and key >= 1 and key <= #expected and key % 1 == 0,
      true,
      context .. ": dense array key"
    )
  end
end

local function lifecycle_authoritative_projection(apply_plans, context)
  local projection = {}
  local grantless_effect_ids = {
    ["consensus.proposal"] = true,
    ["github-proxy.github_issue_comment_request"] = true,
  }
  for _, plan in ipairs(apply_plans) do
    t.eq(plan.dept, "loop", context .. ": apply plan department")
    t.eq(plan.to_state, nil, context .. ": no state:v1 marker write")
    for _, label in ipairs((plan.labels and plan.labels.add) or {}) do
      if devloop_state.is_state_label(label) then
        table.insert(projection, "label:add:" .. label)
      end
    end
    for _, label in ipairs((plan.labels and plan.labels.remove) or {}) do
      if devloop_state.is_state_label(label) then
        table.insert(projection, "label:remove:" .. label)
      end
    end
    for _, effect_id in ipairs(plan.effect_ids) do
      t.eq(
        grantless_effect_ids[effect_id],
        true,
        context .. ": observed effect is published-seam or non-lifecycle grantless"
      )
      if grantless_effect_ids[effect_id] ~= true then
        table.insert(projection, effect_id)
      end
    end
  end
  return projection
end

local function assert_case(fixture)
  local event = h.unresolved({
    dedup_key = V_CURRENT,
    round = 0,
    narrowed_question = "Which fact resolves the remaining gap?",
    angle_digests = {
      { angle = "minimal", verdict = "abstain", digest = "shadow-parity" },
    },
    essence_stall = fixture.essence_stall,
  })
  local labels = { "fkst-dev:enabled" }
  if fixture.current_state ~= nil then
    table.insert(labels, state_labels[fixture.current_state])
  end
  h.mock_issue_loop(labels, fixture_comments(event, fixture))
  if fixture.needs_context then
    h.mock_context_bundle(event)
  end

  local result, probes, decisions, apply_plans = observe_department(function()
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
  t.eq(#apply_plans, fixture.expected_status == "apply" and 1 or 0, fixture.name .. ": apply plan count")
  if #apply_plans == 1 then
    assert_array(
      apply_plans[1].effect_ids,
      fixture.expected_effect_ids,
      fixture.name .. ": observed OLD effect ids"
    )
  end
  assert_array(
    lifecycle_authoritative_projection(apply_plans, fixture.name),
    {},
    fixture.name .. ": lifecycle-authoritative projection"
  )

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
  t.eq(shadow.status, fixture.expected_status, fixture.name .. ": expected shadow status")
  assert_bidirectional(shadow, legacy, "reason_code", fixture.name)
  assert_bidirectional(shadow, legacy, "cas_outcome", fixture.name)
  t.eq(shadow.edge_id, EDGE_ID, fixture.name .. ": selected edge id")
  t.eq(shadow.cas_policy_id, POLICY_ID, fixture.name .. ": selected CAS policy")
  t.eq(shadow.evidence.status, "complete", fixture.name .. ": evidence status")
  t.eq(#shadow.evidence.refs, 0, fixture.name .. ": default evidence refs")
  t.eq(shadow.evidence.facts.source, "thinking", fixture.name .. ": evidence source")
  t.eq(shadow.evidence.facts.target, "blocked", fixture.name .. ": evidence target")
  t.eq(shadow.effect_entitlement_id, fixture.expected_entitlement_id, fixture.name .. ": entitlement id")
  if fixture.expected_entitlement_id ~= nil then
    assert_array(shadow.granted_effect_ids, {}, fixture.name .. ": granted effect ids")
  else
    t.eq(shadow.granted_effect_ids, nil, fixture.name .. ": no granted effect ids")
  end
  t.eq(shadow.grant, nil, fixture.name .. ": grant disabled")
  return shadow
end

local function assert_consensus_result_case(fixture)
  local event = h.reached({ effect_version = fixture.incoming_version })
  local labels = { "fkst-dev:enabled" }
  if fixture.current_state ~= nil then
    table.insert(labels, state_labels[fixture.current_state])
  end
  local comments = {}
  if fixture.current_state ~= nil then
    table.insert(comments, core.state_marker(
      event.proposal_id,
      fixture.current_state,
      fixture.current_version
    ))
  end
  h.mock_issue_result(labels, comments)

  t.eq(
    type(consensus_result_department.make_department),
    "function",
    fixture.name .. ": real consensus_result department loaded"
  )
  local result, probes, decisions = observe_consensus_result_department(function()
    local runner = fixture.expected_exit_code == 1
      and h.run_result_expecting_failure
      or h.run_result
    return runner(event, h.opts("restart-authority-versioned-shadow-parity"))
  end)

  t.eq(result.exit_code, fixture.expected_exit_code, fixture.name .. ": department exit code")
  t.eq(#probes, 1, fixture.name .. ": real department CAS probe count")
  local probe = probes[1]
  t.eq(probe.current.state, fixture.current_state, fixture.name .. ": observed current state")
  t.eq(probe.current.version, fixture.current_version, fixture.name .. ": observed current version")
  t.eq(probe.from_states[1], "thinking", fixture.name .. ": observed source state")
  t.eq(#probe.from_states, 1, fixture.name .. ": observed source state count")
  t.eq(probe.to_state, "ready", fixture.name .. ": observed target state")
  t.eq(probe.incoming_version, event.effect_version, fixture.name .. ": observed incoming version")
  for _, decision in ipairs(decisions) do
    t.eq(decision.dept, "consensus_result", fixture.name .. ": legacy decision department")
    t.eq(decision.from_state, "thinking", fixture.name .. ": legacy decision source")
    t.eq(decision.to_state, "ready", fixture.name .. ": legacy decision target")
  end

  local legacy = observed_consensus_result_admission(probe, decisions)
  local sealed = restart_authority.seal_snapshot({
    owner = OWNER,
    proposal_id = event.proposal_id,
    current = {
      state = fixture.current_state,
      version = fixture.current_version,
    },
  })
  local shadow = restart_authority.decide_transition(sealed, {
    semantic_variant = CONSENSUS_REACHED_VARIANT,
    incoming_version = event.effect_version,
  })

  assert_bidirectional(shadow, legacy, "status", fixture.name)
  t.eq(shadow.status, fixture.expected_status, fixture.name .. ": expected shadow status")
  assert_bidirectional(shadow, legacy, "reason_code", fixture.name)
  assert_bidirectional(shadow, legacy, "cas_outcome", fixture.name)
  t.eq(shadow.edge_id, CONSENSUS_REACHED_EDGE_ID, fixture.name .. ": selected edge id")
  t.eq(shadow.cas_policy_id, CONSENSUS_RESULT_POLICY_ID, fixture.name .. ": selected CAS policy")
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
    local entitlement_ids = {}
    for _, fixture in ipairs(fixtures) do
      local shadow = assert_case(fixture)
      if shadow.effect_entitlement_id ~= nil then
        entitlement_ids[shadow.status] = shadow.effect_entitlement_id
      end
    end
    t.eq(entitlement_ids.apply, APPLY_ENTITLEMENT_ID, "apply entitlement selected")
    t.eq(entitlement_ids.idempotent, IDEMPOTENT_ENTITLEMENT_ID, "idempotent entitlement selected")
    t.eq(entitlement_ids.apply == entitlement_ids.idempotent, false, "apply and idempotent entitlement ids are distinct")
  end,

  test_shadow_decider_matches_legacy_consensus_result_versioned_cas_triplets = function()
    for _, fixture in ipairs(consensus_result_fixtures) do
      assert_consensus_result_case(fixture)
    end
  end,

  test_shadow_decider_requires_versioned_incoming_version = function()
    local missing = restart_authority.decide_transition(sealed_snapshot(), {
      semantic_variant = CONSENSUS_REACHED_VARIANT,
    })
    assert_illegal(
      missing,
      "incoming-version-required",
      "illegal(incoming-version-required)",
      "missing versioned incoming version"
    )

    local malformed_versions = {
      {
        context = "empty incoming version",
        intent = { semantic_variant = CONSENSUS_REACHED_VARIANT, incoming_version = "" },
      },
      {
        context = "non-string incoming version",
        intent = { semantic_variant = CONSENSUS_REACHED_VARIANT, incoming_version = 42 },
      },
      {
        context = "empty target version",
        intent = {
          semantic_variant = CONSENSUS_REACHED_VARIANT,
          incoming_version = V_CURRENT,
          target_version = "",
        },
      },
      {
        context = "non-string target version",
        intent = {
          semantic_variant = CONSENSUS_REACHED_VARIANT,
          incoming_version = V_CURRENT,
          target_version = 42,
        },
      },
    }
    for _, fixture in ipairs(malformed_versions) do
      assert_illegal(
        restart_authority.decide_transition(sealed_snapshot(), fixture.intent),
        "malformed-intent",
        "illegal(malformed-intent)",
        fixture.context
      )
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
