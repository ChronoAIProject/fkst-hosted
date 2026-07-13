-- Non-circularity contract: production truth comes from the real implement
-- department's named CAS wrapper, the exact inner probe arguments it produces,
-- and the first post-admission WIP guard. Catalog evidence is copied from those
-- observed arguments, never reconstructed from fixture versions. Handoff
-- verification, effects, liveness dedup, and legacy logs remain separate axes.

local catalog = require("devloop.restart_cas_catalog")
local owner_pending_projection = require("devloop.restart_owner_pending_projection")
local inventories = {
  canonicalization = require("core.restart.canonicalization_inventory"),
  entry = require("core.restart.entry_inventory"),
  operator_reentry = require("core.restart.operator_reentry_inventory"),
}
local devloop_logging = require("devloop.logging")
local devloop_state = require("devloop.state")
local dispatch_live_run = require("devloop.dispatch_live_run")
local m_builders = require("devloop.markers.builders")
local m_mq = require("devloop.merge_queue")
local payloads_predicates = require("devloop.payloads.predicates")
local transitions = require("departments.implement.transitions")
local h = require("tests.devloop_helpers")
local t = h.t
local core = h.core
local projection = owner_pending_projection.derive(core.restart_package_name, core.restart_transition_table(), inventories)
local implement_department = require("departments.implement.main")

local POLICY_ID = "cas.legacy_implement_activation_handoff_v1"
local REPO = "owner/repo"
local ISSUE_NUMBER = 42
local WIP_ISSUE_NUMBER = 51
local PROPOSAL_ID = "github-devloop/issue/owner/repo/42"
local INTEGRATION_BRANCH = "integration/dev"
local V_OLDER = "ready/consensus-github-devloop/issue/owner/repo/42/2026-06-02T01-02-03Z"
local V_EQUAL = "ready/consensus-github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z"
local V_NEWER = "ready/consensus-github-devloop/issue/owner/repo/42/2026-06-04T01-02-03Z"
local V_ORDERING_EQUAL_CURRENT = V_EQUAL .. "/loop/01"
local V_ORDERING_EQUAL_INCOMING = V_EQUAL .. "/loop/1"

local variants = {
  ["ready\0implementing"] = "ready_to_implementing",
  ["impl-failed\0implementing"] = "impl_failed_to_implementing",
  ["blocked\0implementing"] = "blocked_to_implementing",
}

local function source_state_names(expected_states)
  local names = {}
  for _, expected in ipairs(expected_states or {}) do
    table.insert(names, type(expected) == "table" and expected.state or expected)
  end
  return names
end

local function probe_variant(from_states, to_state)
  if type(from_states) ~= "table" or #from_states ~= 1 then
    return nil
  end
  return variants[tostring(from_states[1]) .. "\0" .. tostring(to_state)]
end

local function observe_department(run)
  local named_calls = {}
  local probes = {}
  local decisions = {}
  local handoff_checks = {}
  local boundary_calls = {}
  local sequence = 0
  local active_named_call = nil
  local original_implementation = transitions.implementation_transition_status
  local original_versioned = devloop_state.versioned_transition_status
  local original_cyclic = devloop_state.cyclic_transition_status
  local original_log_cas = devloop_logging.log_cas_decision
  local original_verified_hand_off_state = payloads_predicates.verified_hand_off_state
  local original_wip_capacity_allows_start = m_mq.wip_capacity_allows_start
  local original_dispatch_live_run_dedup = dispatch_live_run.dispatch_live_run_dedup

  dispatch_live_run.dispatch_live_run_dedup = function()
    return false
  end
  transitions.implementation_transition_status = function(state, expected_states, marker_version)
    sequence = sequence + 1
    local named_call = {
      sequence = sequence,
      current = state,
      expected_states = expected_states,
      marker_version = marker_version,
      index = #named_calls + 1,
    }
    table.insert(named_calls, named_call)
    active_named_call = named_call
    local outcome = original_implementation(state, expected_states, marker_version)
    named_call.outcome = outcome
    active_named_call = nil
    return outcome
  end
  devloop_state.versioned_transition_status = function(current, from_states, to_state, incoming_version, target_version)
    local outcome = original_versioned(current, from_states, to_state, incoming_version, target_version)
    sequence = sequence + 1
    table.insert(probes, {
      sequence = sequence,
      named_call = active_named_call,
      kind = "versioned",
      current = current,
      from_states = from_states,
      to_state = to_state,
      incoming_version = incoming_version,
      target_version = target_version,
      outcome = outcome,
      variant = probe_variant(from_states, to_state),
    })
    return outcome
  end
  devloop_state.cyclic_transition_status = function(current, from_states, to_state, incoming_version, target_version)
    local outcome = original_cyclic(current, from_states, to_state, incoming_version, target_version)
    sequence = sequence + 1
    table.insert(probes, {
      sequence = sequence,
      named_call = active_named_call,
      kind = "cyclic",
      current = current,
      from_states = from_states,
      to_state = to_state,
      incoming_version = incoming_version,
      target_version = target_version,
      outcome = outcome,
      variant = probe_variant(from_states, to_state),
    })
    return outcome
  end
  devloop_logging.log_cas_decision = function(dept, proposal_id, current, from_state, to_state, outcome, reason)
    sequence = sequence + 1
    table.insert(decisions, {
      sequence = sequence,
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
  payloads_predicates.verified_hand_off_state = function(M, repo, hand_off, expected)
    local state, reason = original_verified_hand_off_state(M, repo, hand_off, expected)
    sequence = sequence + 1
    table.insert(handoff_checks, {
      sequence = sequence,
      state = state,
      reason = reason,
      repo = repo,
      hand_off = hand_off,
      expected = expected,
    })
    return state, reason
  end
  m_mq.wip_capacity_allows_start = function(M, repo, current_issue_number)
    local allowed, reason, count, maximum = original_wip_capacity_allows_start(M, repo, current_issue_number)
    sequence = sequence + 1
    table.insert(boundary_calls, {
      sequence = sequence,
      repo = repo,
      issue_number = current_issue_number,
      allowed = allowed,
      reason = reason,
      count = count,
      maximum = maximum,
    })
    return allowed, reason, count, maximum
  end

  local ok, result = pcall(run)
  dispatch_live_run.dispatch_live_run_dedup = original_dispatch_live_run_dedup
  m_mq.wip_capacity_allows_start = original_wip_capacity_allows_start
  payloads_predicates.verified_hand_off_state = original_verified_hand_off_state
  devloop_logging.log_cas_decision = original_log_cas
  devloop_state.cyclic_transition_status = original_cyclic
  devloop_state.versioned_transition_status = original_versioned
  transitions.implementation_transition_status = original_implementation
  if not ok then
    error(result, 0)
  end
  return result, named_calls, probes, decisions, handoff_checks, boundary_calls
end

local function evidence_from_probe(probe, handoff_checks)
  local definition = catalog.definition(POLICY_ID)
  local variant = definition and definition.variants[probe.variant]
  t.is_true(variant ~= nil, "observed implement probe must select a catalog variant")
  t.eq(#probe.from_states, #variant.source_states, "catalog source-state count comes from probe signature")
  for index, source_state in ipairs(probe.from_states) do
    t.eq(variant.source_states[index], source_state, "catalog source state comes from probe signature")
  end
  t.eq(variant.target_state, probe.to_state, "catalog target state comes from probe signature")

  local handoff = nil
  if #handoff_checks > 0 then
    handoff = { status = handoff_checks[#handoff_checks].state ~= nil and "valid" or "invalid" }
  end
  return {
    current = probe.current,
    variant = probe.variant,
    incoming_version = probe.incoming_version,
    target_version = probe.target_version,
    phase = probe.named_call and probe.named_call.index == 1 and "initial" or "recheck",
    retry = probe.variant ~= "ready_to_implementing",
    handoff = handoff,
  }
end

local function decision_after_probe(decisions, probe, boundary_calls)
  local boundary_sequence = boundary_calls[1] and boundary_calls[1].sequence or math.huge
  for _, decision in ipairs(decisions) do
    if decision.sequence > probe.sequence
      and decision.sequence < boundary_sequence
      and decision.dept == "implement"
      and decision.to_state == "implementing" then
      return decision
    end
  end
  return nil
end

local function observed_admission(probe, decision, handoff_checks, boundary_reached)
  if boundary_reached then
    if probe.outcome == "pending" then
      local handoff = handoff_checks[#handoff_checks]
      t.is_true(handoff ~= nil and handoff.state ~= nil, "pending probe reaches boundary only through verified handoff")
      return { status = "apply", reason_code = "verified-own-ready-hand-off" }
    end
    t.eq(probe.outcome, "apply", "implement admission boundary requires applying CAS")
    return { status = "apply", reason_code = "apply" }
  end
  if probe.outcome == "pending" then
    return { status = "pending", reason_code = "source-marker-not-visible" }
  end
  if probe.outcome == "idempotent" then
    return { status = "idempotent", reason_code = "already-at-target" }
  end
  if probe.outcome == "stale" then
    local outcome = tostring(decision and decision.outcome or "")
    if outcome:find("incoming version < current marker version", 1, true) ~= nil then
      return { status = "stale", reason_code = "incoming-version-older" }
    end
    return { status = "stale", reason_code = "advanced-or-diverged" }
  end
  error("implement CAS probe applied without reaching the admission boundary")
end

local function post_admission_disposition(boundary_calls)
  if #boundary_calls == 0 then
    return "not-admitted"
  end
  local boundary = boundary_calls[1]
  if not boundary.allowed and boundary.reason == "wip-cap-reached" then
    return "wip-deferred"
  end
  return "admitted-past-wip"
end

local function render_comment(body)
  return string.format(
    '{"body":"%s","author":{"login":"fkst-test-bot"},"createdAt":"2026-06-03T01:00:00Z"}',
    tostring(body or ""):gsub("\\", "\\\\"):gsub('"', '\\"'):gsub("\n", "\\n")
  )
end

local function mock_wip_stop()
  t.mock_command('printf %s "$FKST_DEVLOOP_MAX_INFLIGHT"', {
    stdout = "1", stderr = "", exit_code = 0,
  })
  t.mock_command('printf %s "$FKST_DEVLOOP_UPSTREAM_BRANCH"', {
    stdout = "dev", stderr = "", exit_code = 0,
  })
  t.mock_command('printf %s "$FKST_DEVLOOP_INTEGRATION_BRANCH"', {
    stdout = INTEGRATION_BRANCH, stderr = "", exit_code = 0,
  })
  t.mock_command(core.gh_issue_list_wip_cmd(REPO), {
    stdout = '[{"number":' .. tostring(WIP_ISSUE_NUMBER) .. '}]\n', stderr = "", exit_code = 0,
  })
  local holder_proposal = "github-devloop/issue/owner/repo/" .. tostring(WIP_ISSUE_NUMBER)
  local holder_version = "ready/consensus-github-devloop/issue/owner/repo/51/2026-06-03T01-02-03Z"
  t.mock_command(core.gh_issue_view_state_cmd(REPO, WIP_ISSUE_NUMBER), {
    stdout = string.format(
      '{"title":"WIP","state":"OPEN","labels":[{"name":"fkst-dev:implementing"}],"comments":[%s],"assignees":[{"login":"fkst-test-bot"}],"author":{"login":"fkst-test-bot"}}\n',
      render_comment(core.state_marker(holder_proposal, "implementing", holder_version))
    ),
    stderr = "",
    exit_code = 0,
  })
end

local function make_event(fixture)
  local event = h.ready({ dedup_key = fixture.event_version or V_EQUAL })
  if fixture.impl_retry_attempt ~= nil then
    event.impl_retry_attempt = fixture.impl_retry_attempt
  end
  if fixture.handoff_visible_version ~= nil then
    event.ready_hand_off = {
      kind = "own-state-marker",
      proposal_id = event.proposal_id,
      state = "ready",
      marker_version = event.dedup_key,
      event_version = event.dedup_key,
      stage_rank = core.stage_rank("ready"),
      effects = "result-marker,ready-label,devloop-ready",
      comment_id = "IC_implement_cas_handoff",
    }
  end
  if fixture.blocked_version ~= nil then
    event.operator_reentry = {
      command = "reimplement",
      from_state = "blocked",
      pr_number = 7,
      state_version = fixture.blocked_version,
      impl_version = event.dedup_key,
    }
  end
  return event
end

local function fixture_comments(fixture, event)
  local comments = {}
  if fixture.current_state ~= nil then
    table.insert(comments, core.state_marker(PROPOSAL_ID, fixture.current_state, fixture.current_version))
  end
  if fixture.impl_failure then
    table.insert(comments, core.impl_failure_marker(PROPOSAL_ID, event.dedup_key, "codex-failed", 1))
  end
  if fixture.blocked_version ~= nil or fixture.current_target_link then
    table.insert(comments, m_builders.pr_link_marker(
      PROPOSAL_ID,
      7,
      "devloop-owner-repo-42-01HY",
      event.dedup_key,
      "dev"
    ))
  end
  return comments
end

local function mock_case(fixture, event)
  h.mock_bot_env()
  h.mock_write_env("")
  t.mock_command("gh api graphql", {
    stdout = '{"data":{"repository":{"issue":{"blockedBy":{"nodes":[]}}}}}\n',
    stderr = "",
    exit_code = 0,
  })
  h.mock_issue_implement_raw(
    { "fkst-dev:" .. tostring(fixture.current_state or "ready") },
    fixture_comments(fixture, event)
  )
  mock_wip_stop()
  if fixture.handoff_visible_version ~= nil then
    local visible_marker = core.state_marker(PROPOSAL_ID, "ready", fixture.handoff_visible_version)
    t.mock_command("gh api --method GET 'repos/owner/repo/issues/comments/IC_implement_cas_handoff'", {
      stdout = '{"body":"' .. h.json_string(visible_marker) .. '","user":{"login":"fkst-test-bot"}}\n',
      stderr = "",
      exit_code = 0,
    })
  end
end

local function run_real_department(event)
  local raises = {}
  local original_raise = raise
  raise = function(queue, payload)
    table.insert(raises, { queue = queue, payload = payload })
  end
  local ok, failure = pcall(implement_department.pipeline, {
    queue = "devloop_ready",
    payload = event,
  })
  raise = original_raise
  return {
    exit_code = ok and 0 or 1,
    error = ok and nil or tostring(failure),
    raises = raises,
  }
end

local function assert_case(fixture)
  local event = make_event(fixture)
  mock_case(fixture, event)
  local result, named_calls, probes, decisions, handoff_checks, boundary_calls = observe_department(function()
    return run_real_department(event)
  end)

  local admission_phase = #probes == 0 and "pre-cas" or "cas"
  t.eq(admission_phase, fixture.admission_phase or "cas", fixture.name .. ": admission phase")
  t.eq(#named_calls, #probes, fixture.name .. ": named wrapper and exact probe counts agree")
  t.eq(#probes, admission_phase == "cas" and 1 or 0, fixture.name .. ": exact CAS probe count")
  t.eq(#boundary_calls, fixture.boundary_reached and 1 or 0, fixture.name .. ": WIP admission boundary reach")

  if admission_phase == "cas" then
    local probe = probes[1]
    t.is_true(probe.named_call ~= nil, fixture.name .. ": exact probe is nested under named production function")
    t.eq(probe.named_call.current, probe.current, fixture.name .. ": wrapper current is exact probe current")
    t.eq(probe.to_state, "implementing", fixture.name .. ": exact probe target")
    local evidence = evidence_from_probe(probe, handoff_checks)
    t.eq(evidence.current, probe.current, fixture.name .. ": catalog current comes from probe")
    t.eq(evidence.incoming_version, probe.incoming_version, fixture.name .. ": catalog incoming version comes from probe")
    t.eq(evidence.target_version, probe.target_version, fixture.name .. ": catalog target version comes from probe")
    if fixture.probe_incoming_differs_from_event then
      t.is_true(
        tostring(probe.incoming_version) ~= tostring(event.dedup_key),
        fixture.name .. ": production-derived probe version differs from raw event"
      )
    end

    local decision = decision_after_probe(decisions, probe, boundary_calls)
    t.is_true(decision ~= nil, fixture.name .. ": legacy CAS decision captured separately")
    local observed = observed_admission(probe, decision, handoff_checks, #boundary_calls == 1)
    local actual = catalog.resolve(POLICY_ID, evidence, projection)
    t.eq(actual.status, observed.status, fixture.name .. ": admission status parity")
    t.eq(actual.reason_code, observed.reason_code, fixture.name .. ": admission reason parity")
    if fixture.admission_status ~= nil then
      t.eq(observed.status, fixture.admission_status, fixture.name .. ": observed admission status")
      t.eq(actual.status, fixture.admission_status, fixture.name .. ": catalog admission status")
    end
    if fixture.legacy_log_outcome ~= nil then
      t.eq(decision.outcome, fixture.legacy_log_outcome, fixture.name .. ": legacy log outcome")
    end
  else
    t.eq(#boundary_calls, 0, fixture.name .. ": pre-CAS input cannot reach admission boundary")
  end

  t.eq(result.exit_code, fixture.expected_exit_code or 0, fixture.name .. ": department exit code")
  t.eq(#result.raises, fixture.effect_count or 0, fixture.name .. ": captured effects")
  t.eq(
    post_admission_disposition(boundary_calls),
    fixture.post_admission_disposition or "not-admitted",
    fixture.name .. ": post-admission disposition"
  )
end

local function assert_malformed_fail_closed()
  local payload = h.ready()
  payload.dedup_key = 42
  local result, named_calls, probes, _, _, boundary_calls = observe_department(function()
    return run_real_department(payload)
  end)
  t.eq(result.exit_code, 0, "malformed payload exits through the production pre-CAS guard")
  t.eq(#named_calls, 0, "malformed payload does not call the named CAS function")
  t.eq(#probes, 0, "malformed payload does not reach an exact CAS probe")
  t.eq(#boundary_calls, 0, "malformed payload does not reach the admission boundary")
  local resolved = catalog.resolve(POLICY_ID, {
    current = { state = "ready", version = V_EQUAL },
    variant = "ready_to_implementing",
    incoming_version = payload.dedup_key,
    phase = "initial",
    retry = false,
  }, projection)
  t.eq(resolved.status, "illegal", "catalog rejects the same malformed version value")
  t.eq(resolved.reason_code, "invalid-evidence", "catalog malformed reason")
  t.eq(resolved.cas_outcome, "illegal(invalid-evidence)", "catalog malformed fail-closed outcome")
end

return {
  test_ready_source_equal_reaches_admission_boundary = function()
    assert_case({
      name = "ready-source-equal",
      current_state = "ready",
      current_version = V_EQUAL,
      event_version = V_EQUAL,
      boundary_reached = true,
      admission_status = "apply",
      legacy_log_outcome = "applied",
      post_admission_disposition = "wip-deferred",
    })
  end,

  test_ready_source_older_event_is_stale = function()
    assert_case({
      name = "ready-source-older",
      current_state = "ready",
      current_version = V_EQUAL,
      event_version = V_OLDER,
      admission_status = "stale",
      legacy_log_outcome = "skip-stale(incoming version < current marker version)",
    })
  end,

  test_earlier_state_with_newer_event_is_pending = function()
    assert_case({
      name = "thinking-source-newer",
      current_state = "thinking",
      current_version = V_EQUAL,
      event_version = V_NEWER,
      admission_status = "pending",
      expected_exit_code = 1,
      legacy_log_outcome = "retry-pending(from-state marker not yet visible)",
    })
  end,

  test_unrelated_current_state_is_stale = function()
    assert_case({
      name = "blocked-unrelated-stale",
      current_state = "blocked",
      current_version = V_EQUAL,
      event_version = V_EQUAL,
      admission_status = "stale",
      legacy_log_outcome = "skip-advanced-or-diverged",
    })
  end,

  test_ordering_equal_raw_different_versions_use_observed_probe_evidence = function()
    t.is_true(
      V_ORDERING_EQUAL_CURRENT ~= V_ORDERING_EQUAL_INCOMING,
      "ordering-equal fixture versions must be byte-different"
    )
    assert_case({
      name = "ready-ordering-equal-raw-different",
      current_state = "ready",
      current_version = V_ORDERING_EQUAL_CURRENT,
      event_version = V_ORDERING_EQUAL_INCOMING,
      boundary_reached = true,
      admission_status = "apply",
      legacy_log_outcome = "applied",
      post_admission_disposition = "wip-deferred",
    })
  end,

  test_valid_direct_id_handoff_overlays_pending_probe_to_apply = function()
    assert_case({
      name = "verified-ready-handoff",
      current_state = nil,
      current_version = nil,
      event_version = V_EQUAL,
      handoff_visible_version = V_EQUAL,
      boundary_reached = true,
      admission_status = "apply",
      legacy_log_outcome = "apply(verified-own-ready-hand-off)",
      post_admission_disposition = "wip-deferred",
    })
  end,

  test_direct_id_handoff_raw_version_mismatch_remains_pending = function()
    assert_case({
      name = "ready-handoff-version-mismatch",
      current_state = nil,
      current_version = nil,
      event_version = V_EQUAL,
      handoff_visible_version = V_OLDER,
      admission_status = "pending",
      expected_exit_code = 1,
      legacy_log_outcome = "retry-pending(from-state marker not yet visible)",
    })
  end,

  test_impl_failed_retry_variant_applies_with_derived_attempt_version = function()
    assert_case({
      name = "impl-failed-retry",
      current_state = "impl-failed",
      current_version = V_EQUAL,
      event_version = V_EQUAL,
      impl_retry_attempt = 2,
      impl_failure = true,
      boundary_reached = true,
      admission_status = "apply",
      post_admission_disposition = "wip-deferred",
    })
  end,

  test_blocked_reentry_variant_uses_spied_cyclic_versions = function()
    local blocked_version = V_EQUAL .. "/review-loop/3"
    assert_case({
      name = "blocked-operator-reentry",
      current_state = "blocked",
      current_version = blocked_version,
      blocked_version = blocked_version,
      event_version = V_EQUAL,
      impl_retry_attempt = 2,
      boundary_reached = true,
      admission_status = "apply",
      probe_incoming_differs_from_event = true,
      post_admission_disposition = "wip-deferred",
    })
  end,

  test_current_target_is_classified_as_pre_cas = function()
    assert_case({
      name = "implementing-target-pre-cas",
      current_state = "implementing",
      current_version = V_EQUAL,
      event_version = V_EQUAL,
      current_target_link = true,
      admission_phase = "pre-cas",
    })
  end,

  test_malformed_version_fails_closed_before_cas_and_in_catalog = function()
    assert_malformed_fail_closed()
  end,
}
