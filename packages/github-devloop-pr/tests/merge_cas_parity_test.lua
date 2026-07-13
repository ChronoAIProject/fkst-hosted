-- Non-circularity contract: production truth comes from the real merge
-- department's CAS probe and the merge-ready fact admission boundary. Effects
-- and legacy CAS logs are recorded as separate post-admission observations.
-- Catalog evidence comes only from the captured probe arguments, and this test
-- never computes the expected result with a devloop.state transition helper.

local catalog = require("devloop.restart_cas_catalog")
local owner_pending_projection = require("devloop.restart_owner_pending_projection")
local inventories = {
  canonicalization = require("core.restart.canonicalization_inventory"),
  entry = require("core.restart.entry_inventory"),
  operator_reentry = require("core.restart.operator_reentry_inventory"),
}
local devloop_logging = require("devloop.logging")
local m_builders = require("devloop.markers.builders")
local m_facts = require("devloop.markers.facts")
local devloop_state = require("devloop.state")
local h = require("tests.devloop_helpers")
local t = h.t
local core = h.core
local projection = owner_pending_projection.derive(core.restart_package_name, core.restart_transition_table(), inventories)
local merge_department = require("departments.merge.main")

local POLICY_ID = "cas.legacy_merge_v1"
local VARIANT = "merge_ready_or_merging_to_merging"
local V_OLDER = "ready/consensus-github-devloop/issue/owner/repo/42/2026-06-02T01-02-03Z"
local V_EQUAL = "ready/consensus-github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z"
local V_NEWER = "ready/consensus-github-devloop/issue/owner/repo/42/2026-06-04T01-02-03Z"
local V_ORDERING_EQUAL_CURRENT = V_EQUAL .. "/loop/01"
local V_ORDERING_EQUAL_INCOMING = V_EQUAL .. "/loop/1"

local function observe_department(run)
  local probes = {}
  local decisions = {}
  local boundary_calls = {}
  local original_cyclic = devloop_state.cyclic_transition_status
  local original_log_cas = devloop_logging.log_cas_decision
  local original_merge_ready_fact = m_facts.merge_ready_fact

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
  m_facts.merge_ready_fact = function(comments, proposal_id, version, pr_number, head_sha)
    table.insert(boundary_calls, {
      comments = comments,
      proposal_id = proposal_id,
      version = version,
      pr_number = pr_number,
      head_sha = head_sha,
    })
    return original_merge_ready_fact(comments, proposal_id, version, pr_number, head_sha)
  end

  local ok, result = pcall(run)
  m_facts.merge_ready_fact = original_merge_ready_fact
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

local function evidence_from_probe(probe)
  return {
    current = current_fact(probe.current.state, probe.current.version),
    variant = VARIANT,
    incoming_version = probe.incoming_version,
    target_version = probe.target_version,
    overlay_version = probe.incoming_version,
  }
end

local function observed_admission(probe, decision, boundary_reached)
  local legacy_outcome = tostring(decision and decision.outcome or "")
  if not boundary_reached and legacy_outcome:find("from-state-mismatch", 1, true) ~= nil then
    return { status = "stale", reason_code = "from-state-mismatch" }
  end
  if not boundary_reached and legacy_outcome:find("version-mismatch", 1, true) ~= nil then
    return { status = "stale", reason_code = "version-mismatch" }
  end
  if probe.outcome == "pending" then
    return { status = "pending", reason_code = "source-marker-not-visible" }
  end
  if probe.outcome == "stale" then
    if tostring(probe.incoming_version or "") ~= tostring(probe.current.version or "") then
      return { status = "stale", reason_code = "incoming-version-older" }
    end
    return { status = "stale", reason_code = "advanced-or-diverged" }
  end

  if probe.outcome == "idempotent" then
    return { status = "idempotent", reason_code = "already-at-target" }
  end
  if probe.outcome ~= "apply" then
    error("merge admission probe returned an unknown outcome: " .. tostring(probe.outcome))
  end
  if boundary_reached then
    return { status = "apply", reason_code = "apply" }
  end
  error("merge admission apply did not reach a classified guard")
end

local function post_admission_disposition(result, decision, boundary_reached)
  if not boundary_reached then
    return "not-admitted"
  end
  if #(result.raises or {}) > 0 then
    return "effect-emitted"
  end
  local outcome = tostring(decision and decision.outcome or "")
  if outcome:find("merge-ready fact marker not visible", 1, true) ~= nil then
    return "merge-ready-fact-pending"
  end
  return "post-admission-no-effect"
end

local function run_real_department(event)
  local raises = {}
  local original_raise = raise
  raise = function(queue, payload)
    table.insert(raises, { queue = queue, payload = payload })
  end
  local ok, failure = pcall(merge_department.pipeline, {
    queue = "devloop_merge_ready",
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
  h.mock_write_env("")
  h.mock_default_issue_claim()
  h.mock_pr_merge(comments)
end

local function assert_catalog_matches_observed_decision(fixture)
  local event = h.merge_ready({ version = fixture.incoming_version })
  mock_current_pr(event, fixture)

  local result, probes, decisions, boundary_calls = observe_department(function()
    return run_real_department(event)
  end)

  t.eq(#probes, 1, fixture.name .. ": real department CAS probe count")
  local probe = probes[1]
  t.eq(probe.current.state, fixture.current_state, fixture.name .. ": probe current state")
  t.eq(probe.current.version, fixture.current_version, fixture.name .. ": probe current version")
  t.eq(probe.from_states[1], "merge-ready", fixture.name .. ": first probe source state")
  t.eq(probe.from_states[2], "merging", fixture.name .. ": second probe source state")
  t.eq(#probe.from_states, 2, fixture.name .. ": probe source state count")
  t.eq(probe.to_state, "merging", fixture.name .. ": probe target state")
  t.eq(probe.incoming_version, fixture.incoming_version, fixture.name .. ": probe incoming version")
  t.eq(probe.target_version, nil, fixture.name .. ": probe target version")

  t.eq(#decisions, 1, fixture.name .. ": structured CAS decision count")
  local decision = decisions[1]
  t.eq(decision.dept, "merge", fixture.name .. ": CAS decision department")
  t.eq(decision.from_state, "merge-ready", fixture.name .. ": logged source state")
  t.eq(decision.to_state, "merging", fixture.name .. ": logged target state")
  t.is_true(type(decision.outcome) == "string", fixture.name .. ": legacy log outcome captured")
  t.is_true(type(decision.reason) == "string", fixture.name .. ": legacy log reason captured")

  local boundary_reached = #boundary_calls > 0
  t.eq(#boundary_calls, fixture.boundary_reached and 1 or 0, fixture.name .. ": admission boundary reach")
  if boundary_reached then
    t.eq(boundary_calls[1].proposal_id, event.proposal_id, fixture.name .. ": boundary proposal")
    t.eq(boundary_calls[1].version, event.version, fixture.name .. ": boundary version")
    t.eq(boundary_calls[1].pr_number, event.pr_number, fixture.name .. ": boundary PR")
    t.eq(boundary_calls[1].head_sha, event.reviewed_head_sha, fixture.name .. ": boundary head")
  end

  local observed = observed_admission(probe, decision, boundary_reached)
  local evidence = evidence_from_probe(probe)
  t.eq(evidence.current.state, probe.current.state, fixture.name .. ": catalog current state comes from probe")
  t.eq(evidence.current.version, probe.current.version, fixture.name .. ": catalog current version comes from probe")
  t.eq(evidence.incoming_version, probe.incoming_version, fixture.name .. ": catalog incoming version comes from probe")
  t.eq(evidence.target_version, probe.target_version, fixture.name .. ": catalog target version comes from probe")
  t.eq(evidence.overlay_version, probe.incoming_version, fixture.name .. ": catalog overlay version comes from probe")
  local actual = catalog.resolve(POLICY_ID, evidence, projection)
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
end

local function assert_pre_cas_rejection(name, payload, expected_reason_code)
  local result, probes, _, boundary_calls = observe_department(function()
    return run_real_department(payload)
  end)
  t.eq(result.exit_code, 0, name .. ": unsupported payload is rejected without a pipeline error")
  t.eq(#probes, 0, name .. ": invalid production input must not reach CAS")
  t.eq(#boundary_calls, 0, name .. ": invalid production input must not reach the admission boundary")
  local evidence = {
    current = current_fact("merge-ready", V_EQUAL),
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

local function assert_pre_cas_merged_marker_idempotency()
  local name = "merge-terminal-marker-idempotency"
  local event = h.merge_ready({ version = V_EQUAL })
  local comments = {
    core.state_marker(event.proposal_id, "merged", event.version),
    m_builders.merged_marker(core, event.proposal_id, event.pr_number, event.version, event.reviewed_head_sha),
  }
  h.mock_bot_env()
  h.mock_write_env("")
  h.mock_default_issue_claim()
  h.mock_pr_merge(comments)
  local merge_calls_before = h.count_calls("gh pr merge")

  local result, probes, decisions, boundary_calls = observe_department(function()
    return run_real_department(event)
  end)

  t.eq(result.exit_code, 0, name .. ": department exit code")
  t.eq(#probes, 0, name .. ": terminal effect-idempotency precedes the cyclic CAS probe")
  t.eq(#boundary_calls, 0, name .. ": terminal effect-idempotency precedes the admission boundary")
  t.eq(#result.raises, 0, name .. ": no effect emitted")
  t.eq(h.count_calls("gh pr merge"), merge_calls_before, name .. ": merge effect not invoked")
  t.eq(#decisions, 1, name .. ": structured decision count")
  t.eq(decisions[1].current.state, "merged", name .. ": logged current state")
  t.eq(decisions[1].current.version, V_EQUAL, name .. ": logged current version")
  t.eq(decisions[1].outcome, "skip-idempotent(already at to_state)", name .. ": legacy outcome")
  t.eq(decisions[1].reason, "merged marker already visible", name .. ": legacy reason")
  -- Deliberately no catalog.resolve call: the catalog models CAS admission, not
  -- this trusted merged-marker effect-idempotency guard before the CAS probe.
end

return {
  test_merge_source_equal_is_admitted_before_merge_ready_fact_guard = function()
    assert_catalog_matches_observed_decision({
      name = "merge-source-equal",
      current_state = "merge-ready",
      current_version = V_EQUAL,
      incoming_version = V_EQUAL,
      boundary_reached = true,
      expected_exit_code = 1,
      post_admission_disposition = "merge-ready-fact-pending",
      probe_outcome = "apply",
      admission_status = "apply",
      admission_reason_code = "apply",
      legacy_log_outcome = "retry-pending(merge-ready fact marker not visible)",
    })
  end,

  test_merge_target_equal_is_idempotent_before_merge_ready_fact_guard = function()
    assert_catalog_matches_observed_decision({
      name = "merge-target-equal",
      current_state = "merging",
      current_version = V_EQUAL,
      incoming_version = V_EQUAL,
      boundary_reached = true,
      expected_exit_code = 1,
      post_admission_disposition = "merge-ready-fact-pending",
      probe_outcome = "idempotent",
      admission_status = "idempotent",
      admission_reason_code = "already-at-target",
    })
  end,

  test_merge_source_older_is_stale = function()
    assert_catalog_matches_observed_decision({
      name = "merge-source-older",
      current_state = "merge-ready",
      current_version = V_EQUAL,
      incoming_version = V_OLDER,
      probe_outcome = "stale",
      admission_status = "stale",
      admission_reason_code = "incoming-version-older",
    })
  end,

  test_merge_source_newer_is_pending = function()
    assert_catalog_matches_observed_decision({
      name = "merge-source-newer",
      current_state = "merge-ready",
      current_version = V_EQUAL,
      incoming_version = V_NEWER,
      expected_exit_code = 1,
      probe_outcome = "pending",
      admission_status = "pending",
      admission_reason_code = "source-marker-not-visible",
    })
  end,

  test_merge_target_older_is_stale = function()
    assert_catalog_matches_observed_decision({
      name = "merge-target-older",
      current_state = "merging",
      current_version = V_EQUAL,
      incoming_version = V_OLDER,
      probe_outcome = "stale",
      admission_status = "stale",
      admission_reason_code = "incoming-version-older",
    })
  end,

  test_merge_target_newer_is_pending = function()
    assert_catalog_matches_observed_decision({
      name = "merge-target-newer",
      current_state = "merging",
      current_version = V_EQUAL,
      incoming_version = V_NEWER,
      expected_exit_code = 1,
      probe_outcome = "pending",
      admission_status = "pending",
      admission_reason_code = "source-marker-not-visible",
    })
  end,

  test_merge_missing_current_marker_is_stale_from_state_mismatch = function()
    assert_catalog_matches_observed_decision({
      name = "merge-current-missing",
      current_state = nil,
      current_version = nil,
      incoming_version = V_EQUAL,
      probe_outcome = "pending",
      admission_status = "stale",
      admission_reason_code = "from-state-mismatch",
      legacy_log_outcome = "skip-stale(from-state-mismatch)",
    })
  end,

  -- production's admissible-state guard (merge_executor.lua) fires on a missing marker
  -- BEFORE the pending/version branches, so a missing current is stale/from-state-mismatch
  -- regardless of the incoming version, not the pending retry a version-only model implies.
  test_merge_missing_current_older_version_is_stale_from_state_mismatch = function()
    assert_catalog_matches_observed_decision({
      name = "merge-current-missing-older",
      current_state = nil,
      current_version = nil,
      incoming_version = V_OLDER,
      probe_outcome = "pending",
      admission_status = "stale",
      admission_reason_code = "from-state-mismatch",
      legacy_log_outcome = "skip-stale(from-state-mismatch)",
    })
  end,

  test_merge_missing_current_newer_version_is_stale_from_state_mismatch = function()
    assert_catalog_matches_observed_decision({
      name = "merge-current-missing-newer",
      current_state = nil,
      current_version = nil,
      incoming_version = V_NEWER,
      probe_outcome = "pending",
      admission_status = "stale",
      admission_reason_code = "from-state-mismatch",
      legacy_log_outcome = "skip-stale(from-state-mismatch)",
    })
  end,

  test_merge_unrelated_state_is_stale = function()
    assert_catalog_matches_observed_decision({
      name = "merge-unrelated-stale",
      current_state = "blocked",
      current_version = V_EQUAL,
      incoming_version = V_EQUAL,
    })
  end,

  test_merge_merged_without_terminal_fact_is_admissible_then_stale = function()
    assert_catalog_matches_observed_decision({
      name = "merge-merged-without-terminal-fact",
      current_state = "merged",
      current_version = V_EQUAL,
      incoming_version = V_EQUAL,
      probe_outcome = "stale",
      admission_status = "stale",
      admission_reason_code = "advanced-or-diverged",
      legacy_log_outcome = "skip-advanced-or-diverged",
    })
  end,

  test_merge_merged_without_terminal_fact_older_is_stale = function()
    assert_catalog_matches_observed_decision({
      name = "merge-merged-without-terminal-fact-older",
      current_state = "merged",
      current_version = V_EQUAL,
      incoming_version = V_OLDER,
      probe_outcome = "stale",
      admission_status = "stale",
      admission_reason_code = "incoming-version-older",
    })
  end,

  test_merge_merged_without_terminal_fact_newer_is_pending = function()
    assert_catalog_matches_observed_decision({
      name = "merge-merged-without-terminal-fact-newer",
      current_state = "merged",
      current_version = V_EQUAL,
      incoming_version = V_NEWER,
      expected_exit_code = 1,
      probe_outcome = "pending",
      admission_status = "pending",
      admission_reason_code = "source-marker-not-visible",
    })
  end,

  test_merge_non_admissible_predecessor_raw_apply_is_stale_from_state_mismatch = function()
    assert_catalog_matches_observed_decision({
      name = "merge-predecessor-equal",
      current_state = "reviewing",
      current_version = V_EQUAL,
      incoming_version = V_EQUAL,
      probe_outcome = "apply",
      admission_status = "stale",
      admission_reason_code = "from-state-mismatch",
      legacy_log_outcome = "skip-stale(from-state-mismatch)",
    })
  end,

  test_merge_ordering_equal_raw_different_is_stale_version_mismatch = function()
    t.is_true(
      V_ORDERING_EQUAL_CURRENT ~= V_ORDERING_EQUAL_INCOMING,
      "merge-ordering-equal-raw-different: fixture versions must be byte-different"
    )
    assert_catalog_matches_observed_decision({
      name = "merge-ordering-equal-raw-different",
      current_state = "merge-ready",
      current_version = V_ORDERING_EQUAL_CURRENT,
      incoming_version = V_ORDERING_EQUAL_INCOMING,
      probe_outcome = "apply",
      admission_status = "stale",
      admission_reason_code = "version-mismatch",
      legacy_log_outcome = "skip-stale(version-mismatch)",
    })
  end,

  test_merge_idempotent_ordering_equal_raw_different_is_stale_version_mismatch = function()
    assert_catalog_matches_observed_decision({
      name = "merge-idempotent-ordering-equal-raw-different",
      current_state = "merging",
      current_version = V_ORDERING_EQUAL_CURRENT,
      incoming_version = V_ORDERING_EQUAL_INCOMING,
      probe_outcome = "idempotent",
      admission_status = "stale",
      admission_reason_code = "version-mismatch",
      legacy_log_outcome = "skip-stale(version-mismatch)",
    })
  end,

  test_merge_malformed_evidence_and_payload_fail_closed_before_cas = function()
    local payload = h.merge_ready({ version = V_EQUAL })
    payload.version = 42
    assert_pre_cas_rejection("merge-malformed-version", payload, "invalid-evidence")
  end,

  test_merge_terminal_marker_idempotency_is_pre_cas_and_outside_catalog_parity = function()
    assert_pre_cas_merged_marker_idempotency()
  end,
}
