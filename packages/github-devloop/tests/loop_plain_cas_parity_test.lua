-- Non-circularity contract: production truth comes from the real loop
-- department's transition_status probe and first post-CAS round-marker guard.
-- Catalog evidence is copied from observed probe arguments. Effects and legacy
-- CAS logs are separate axes, never inputs to the admission classification.

local catalog = require("devloop.restart_cas_catalog")
local convergence_shared = require("devloop.convergence.shared")
local conv_rounds = require("devloop.convergence.rounds")
local devloop_logging = require("devloop.logging")
local devloop_state = require("devloop.state")
local h = require("tests.devloop_helpers")
local t = h.t
local core = h.core
local loop_department = require("departments.loop.main")

local POLICY_ID = "cas.legacy_loop_plain_v1"
local VARIANT = "thinking_to_blocked"
local V_CURRENT = "consensus:github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z"
local V_DIFFERENT = "consensus:github-devloop/issue/owner/repo/42/2026-06-04T01-02-03Z"

local state_labels = {
  thinking = "fkst-dev:thinking",
  ready = "fkst-dev:ready",
  blocked = "fkst-dev:blocked",
  ["closed-unmerged"] = "fkst-dev:blocked",
}

local function observe_department(run)
  local probes = {}
  local decisions = {}
  local boundary_calls = {}
  local original_transition = devloop_state.transition_status
  local original_log_cas = devloop_logging.log_cas_decision
  local original_converge_round_facts = conv_rounds.converge_round_facts_for_proposal

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
  conv_rounds.converge_round_facts_for_proposal = function(comments, proposal_id)
    table.insert(boundary_calls, {
      comments = comments,
      proposal_id = proposal_id,
    })
    return original_converge_round_facts(comments, proposal_id)
  end

  local ok, result = pcall(run)
  conv_rounds.converge_round_facts_for_proposal = original_converge_round_facts
  devloop_logging.log_cas_decision = original_log_cas
  devloop_state.transition_status = original_transition
  if not ok then
    error(result, 0)
  end
  return result, probes, decisions, boundary_calls
end

local function evidence_from_probe(probe)
  local definition = catalog.definition(POLICY_ID)
  local variant = definition and definition.variants[VARIANT]
  t.is_true(variant ~= nil, "loop plain probe must select a catalog variant")
  t.eq(definition.production.function_name, "transition_status", "catalog production probe name")
  t.eq(#probe.from_states, #variant.source_states, "catalog source-state count comes from probe")
  for index, source_state in ipairs(probe.from_states) do
    t.eq(variant.source_states[index], source_state, "catalog source state comes from probe")
  end
  t.eq(variant.target_state, probe.to_state, "catalog target state comes from probe")
  return {
    current = probe.current,
    variant = VARIANT,
    incoming_version = probe.incoming_version,
    target_version = probe.target_version,
  }
end

local function observed_admission(probe, boundary_reached)
  if boundary_reached then
    t.eq(probe.outcome, "apply", "loop admission boundary requires an applying CAS probe")
    return { status = "apply", reason_code = "apply" }
  end
  if probe.outcome == "pending" then
    return { status = "pending", reason_code = "source-marker-not-visible" }
  end
  if probe.outcome == "idempotent" then
    return { status = "idempotent", reason_code = "already-at-target" }
  end
  if probe.outcome == "stale" then
    return { status = "stale", reason_code = "advanced-or-diverged" }
  end
  error("loop plain CAS probe applied without reaching its admission boundary")
end

local function post_admission_disposition(result, decision, boundary_reached)
  if not boundary_reached then
    return "not-admitted"
  end
  if #(result.raises or {}) > 0 then
    return "effects-emitted(" .. tostring(#result.raises) .. ")"
  end
  if tostring(decision and decision.outcome or ""):find("converge round lineage already advanced", 1, true) then
    return "round-idempotent"
  end
  return "admitted-no-effect"
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
  local comments = {}
  if fixture.current_state ~= nil then
    table.insert(comments, core.state_marker(
      event.proposal_id,
      fixture.current_state,
      fixture.current_version or V_CURRENT
    ))
  end
  if fixture.round_marker_visible then
    table.insert(comments, conv_rounds.converge_round_marker(
      event.proposal_id,
      fixture.current_version or V_CURRENT,
      convergence_shared.source_ref_digest(event.source_ref),
      0,
      fixture.current_version or V_CURRENT,
      event.narrowed_question,
      event.angle_digests
    ))
  end
  return comments
end

local function assert_case(fixture)
  local event = h.unresolved({
    dedup_key = fixture.raw_event_version or V_CURRENT,
    round = 0,
    narrowed_question = "Which fact resolves the remaining gap?",
    angle_digests = {
      { angle = "minimal", verdict = "abstain", digest = "loop-parity" },
    },
  })
  local labels = { "fkst-dev:enabled" }
  if fixture.current_state ~= nil then
    table.insert(labels, state_labels[fixture.current_state] or "fkst-dev:enabled")
  end
  local comments = fixture_comments(event, fixture)
  h.mock_issue_loop(labels, comments)
  if fixture.effect_count ~= nil and fixture.effect_count > 0 then
    h.mock_context_bundle(event)
  end

  local result, probes, decisions, boundary_calls = observe_department(function()
    return run_real_department(event)
  end)

  t.eq(#probes, 1, fixture.name .. ": real department CAS probe count")
  local probe = probes[1]
  t.eq(probe.current.state, fixture.current_state, fixture.name .. ": probe current state")
  t.eq(probe.current.version, fixture.current_version, fixture.name .. ": probe current version")
  t.eq(probe.from_states[1], "thinking", fixture.name .. ": probe source state")
  t.eq(#probe.from_states, 1, fixture.name .. ": probe source state count")
  t.eq(probe.to_state, "blocked", fixture.name .. ": probe target state")
  t.eq(probe.incoming_version, nil, fixture.name .. ": plain probe has no incoming version")
  t.eq(probe.target_version, nil, fixture.name .. ": plain probe has no target version")

  t.eq(#decisions, 1, fixture.name .. ": structured CAS decision count")
  local decision = decisions[1]
  t.eq(decision.dept, "loop", fixture.name .. ": CAS decision department")
  t.eq(decision.from_state, "thinking", fixture.name .. ": logged source state")
  t.eq(decision.to_state, "thinking", fixture.name .. ": logged loop state")
  t.is_true(type(decision.outcome) == "string", fixture.name .. ": legacy log outcome captured")
  t.is_true(type(decision.reason) == "string", fixture.name .. ": legacy log reason captured")

  local boundary_reached = #boundary_calls > 0
  t.eq(#boundary_calls, fixture.boundary_reached and 1 or 0, fixture.name .. ": admission boundary reach")
  if boundary_reached then
    t.eq(boundary_calls[1].proposal_id, event.proposal_id, fixture.name .. ": boundary proposal")
    t.is_true(type(boundary_calls[1].comments) == "table", fixture.name .. ": boundary comments captured")
  end

  local evidence = evidence_from_probe(probe)
  t.eq(evidence.current, probe.current, fixture.name .. ": catalog current comes from probe")
  t.eq(evidence.incoming_version, probe.incoming_version, fixture.name .. ": catalog incoming version comes from probe")
  t.eq(evidence.target_version, probe.target_version, fixture.name .. ": catalog target version comes from probe")
  local observed = observed_admission(probe, boundary_reached)
  local actual = catalog.resolve(POLICY_ID, evidence)
  t.eq(actual.status, observed.status, fixture.name .. ": admission status parity")
  t.eq(actual.reason_code, observed.reason_code, fixture.name .. ": admission reason parity")
  t.eq(observed.status, fixture.admission_status, fixture.name .. ": observed admission status")
  t.eq(actual.status, fixture.admission_status, fixture.name .. ": catalog admission status")

  t.eq(result.exit_code, fixture.expected_exit_code or 0, fixture.name .. ": department exit code")
  t.eq(#result.raises, fixture.effect_count or 0, fixture.name .. ": captured effect count")
  t.eq(
    post_admission_disposition(result, decision, boundary_reached),
    fixture.post_admission_disposition or "not-admitted",
    fixture.name .. ": post-admission disposition"
  )
  t.eq(decision.outcome, fixture.legacy_log_outcome, fixture.name .. ": legacy log outcome")
end

local function assert_malformed_pre_cas()
  local payload = h.unresolved({ dedup_key = 42 })
  local result, probes, decisions, boundary_calls = observe_department(function()
    return run_real_department(payload)
  end)
  t.eq(result.exit_code, 0, "loop-malformed: production rejects malformed payload without error")
  t.eq(#probes, 0, "loop-malformed: production rejects before CAS")
  t.eq(#boundary_calls, 0, "loop-malformed: production rejects before admission boundary")
  t.eq(#decisions, 1, "loop-malformed: pre-CAS decision is logged")
  t.eq(decisions[1].outcome, "skip-foreign(proposal_id)", "loop-malformed: legacy log outcome")
end

return {
  test_loop_plain_source_state_applies_at_round_marker_boundary = function()
    assert_case({
      name = "loop-source-apply",
      current_state = "thinking",
      current_version = V_CURRENT,
      boundary_reached = true,
      admission_status = "apply",
      effect_count = 2,
      post_admission_disposition = "effects-emitted(2)",
      legacy_log_outcome = "applied",
    })
  end,

  test_loop_plain_target_state_is_idempotent = function()
    assert_case({
      name = "loop-target-idempotent",
      current_state = "blocked",
      current_version = V_CURRENT,
      admission_status = "idempotent",
      legacy_log_outcome = "skip-idempotent(already at to_state)",
    })
  end,

  test_loop_plain_older_unmanaged_state_is_pending = function()
    assert_case({
      name = "loop-older-pending",
      current_state = nil,
      current_version = nil,
      admission_status = "pending",
      expected_exit_code = 1,
      legacy_log_outcome = "retry-pending(from-state marker not yet visible)",
    })
  end,

  test_loop_plain_newer_state_is_stale = function()
    assert_case({
      name = "loop-newer-stale",
      current_state = "ready",
      current_version = V_CURRENT,
      admission_status = "stale",
      legacy_log_outcome = "skip-advanced-or-diverged",
    })
  end,

  test_loop_plain_unrelated_current_is_stale = function()
    assert_case({
      name = "loop-unrelated-stale",
      current_state = "closed-unmerged",
      current_version = V_CURRENT,
      admission_status = "stale",
      legacy_log_outcome = "skip-advanced-or-diverged",
    })
  end,

  test_loop_plain_raw_version_mismatch_has_no_overlay = function()
    assert_case({
      name = "loop-version-mismatch-no-overlay",
      current_state = "thinking",
      current_version = V_CURRENT,
      raw_event_version = V_DIFFERENT,
      boundary_reached = true,
      admission_status = "apply",
      effect_count = 2,
      post_admission_disposition = "effects-emitted(2)",
      legacy_log_outcome = "applied",
    })
  end,

  test_loop_plain_post_admission_round_idempotency_is_separate = function()
    assert_case({
      name = "loop-post-admission-round-idempotent",
      current_state = "thinking",
      current_version = V_CURRENT,
      round_marker_visible = true,
      boundary_reached = true,
      admission_status = "apply",
      post_admission_disposition = "round-idempotent",
      legacy_log_outcome = "skip-stale(converge round lineage already advanced)",
    })
  end,

  test_loop_plain_malformed_input_fails_closed_before_cas = function()
    assert_malformed_pre_cas()
  end,
}
