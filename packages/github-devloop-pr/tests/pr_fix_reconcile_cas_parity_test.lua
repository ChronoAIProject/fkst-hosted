-- Non-circularity contract: production truth comes from the real reconcile
-- department's named CAS probe and first post-admission effect builder. Catalog
-- evidence is copied from observed probe arguments, never reconstructed from the
-- fixture. Effects and legacy CAS logs are recorded as separate axes.

local base_ids = require("devloop.base_ids")
local catalog = require("devloop.restart_cas_catalog")
local devloop_logging = require("devloop.logging")
local devloop_state = require("devloop.state")
local fix_rounds = require("core.fix_rounds")
local transition_version = require("contract.transition_version")
local h = require("tests.devloop_helpers")
local t = h.t
local core = h.core

local POLICY_ID = "cas.legacy_pr_fix_reconcile_v1"
local REVIEW_REJECT_VARIANT = "review_reject_to_blocked"
local BOUNDED_FIX_VARIANT = "bounded_fix_to_blocked"

local V_OLDER = "ready/consensus-github-devloop/issue/owner/repo/42/2026-06-02T01-02-03Z/fix/1/fix/2/fix/3"
local V_EQUAL = "ready/consensus-github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z/fix/1/fix/2/fix/3"
local V_NEWER = "ready/consensus-github-devloop/issue/owner/repo/42/2026-06-04T01-02-03Z/fix/1/fix/2/fix/3"
local V_ORDERING_EQUAL_CURRENT = V_EQUAL .. "/loop/01"
local V_ORDERING_EQUAL_INCOMING = V_EQUAL .. "/loop/1"
local V_SAFE_EQUIVALENT_INCOMING = "x/fix/1/fix/2/fix/3"
local V_SAFE_EQUIVALENT_CURRENT = V_SAFE_EQUIVALENT_INCOMING:gsub("/", "-")

local variant_source_states = {
  [REVIEW_REJECT_VARIANT] = { "reviewing", "fixing", "merge-ready", "merging" },
  [BOUNDED_FIX_VARIANT] = { "fixing", "merge-ready", "merging" },
}

-- reconcile captures this boundary in a local at module load. Install the
-- transparent wrapper only while loading the real department, then restore the
-- exported core function. The captured wrapper is inert outside an observation.
local active_boundary_calls = nil
local original_boundary = core.build_fix_reconcile_comment_request
core.build_fix_reconcile_comment_request = function(
  repo,
  issue_number,
  reconcile,
  action,
  reason
)
  if active_boundary_calls ~= nil then
    table.insert(active_boundary_calls, {
      repo = repo,
      issue_number = issue_number,
      reconcile = reconcile,
      action = action,
      reason = reason,
    })
  end
  return original_boundary(repo, issue_number, reconcile, action, reason)
end
local reconcile_department = require("departments.reconcile.main")
core.build_fix_reconcile_comment_request = original_boundary

local function copy_array(values)
  local out = {}
  for _, value in ipairs(values or {}) do
    table.insert(out, value)
  end
  return out
end

local function fix_reconcile_event(incoming_version, variant)
  local event = h.fix_reconcile()
  event.issue_version = incoming_version
  event.round = core.version_fix_round(incoming_version)
  if variant == BOUNDED_FIX_VARIANT then
    event.schema = fix_rounds.MERGE_GATE_SCHEMA
    event.reason_class = fix_rounds.FIX_LOOP_MAX_ROUNDS
    event.bound_head_sha = event.head_sha
    event.dedup_key = base_ids.dedup_key({
      event.schema,
      incoming_version,
      event.reason_class,
    })
  else
    event.dedup_key = "fix-reconcile:" .. tostring(incoming_version)
  end
  return event
end

local function observe_department(run)
  local probes = {}
  local decisions = {}
  local boundary_calls = {}
  local original_versioned = devloop_state.versioned_transition_status
  local original_log_cas = devloop_logging.log_cas_decision

  devloop_state.versioned_transition_status = function(
    current,
    from_states,
    to_state,
    incoming_version,
    target_version
  )
    local outcome = original_versioned(
      current,
      from_states,
      to_state,
      incoming_version,
      target_version
    )
    table.insert(probes, {
      current = current,
      from_states = copy_array(from_states),
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
  active_boundary_calls = boundary_calls
  local ok, result = pcall(run)
  active_boundary_calls = nil
  devloop_logging.log_cas_decision = original_log_cas
  devloop_state.versioned_transition_status = original_versioned
  if not ok then
    error(result, 0)
  end
  return result, probes, decisions, boundary_calls
end

local function evidence_from_probe(probe, variant)
  return {
    current = probe.current,
    variant = variant,
    incoming_version = probe.incoming_version,
    target_version = probe.target_version,
    overlay_version = probe.incoming_version,
  }
end

local function emitted_state(result)
  for _, raised in ipairs(result.raises or {}) do
    if raised.queue == "github-proxy.github_pr_comment_request" then
      return tostring(raised.payload and raised.payload.body or ""):match('state="([^"]+)"')
    end
  end
  return nil
end

local function state_is_in(state_name, states)
  for _, candidate in ipairs(states or {}) do
    if state_name == candidate then
      return true
    end
  end
  return false
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
    error("PR fix reconcile admission probe returned an unknown outcome: " .. tostring(probe.outcome))
  end
  if not state_is_in(probe.current.state, probe.from_states) then
    return { status = "stale", reason_code = "from-state-mismatch" }
  end
  return { status = "stale", reason_code = "version-mismatch" }
end

local function post_admission_disposition(result, boundary_reached)
  if not boundary_reached then
    return "not-admitted"
  end
  local state = emitted_state(result)
  if state ~= nil then
    return "effect-emitted(" .. state .. ")"
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
    queue = "devloop_fix_reconcile",
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
    table.insert(comments, core.state_marker(
      event.proposal_id,
      fixture.current_state,
      fixture.current_version
    ))
  end
  h.mock_bot_env()
  h.mock_default_issue_claim()
  h.mock_pr_origin(
    comments,
    "devloop-owner-repo-42-01HY",
    fixture.current_head_sha or event.head_sha,
    "OPEN",
    "dev"
  )
end

local function assert_probe_shape(name, probe, variant, fixture)
  t.eq(probe.current.state, fixture.current_state, name .. ": probe current state")
  t.eq(probe.current.version, fixture.current_version, name .. ": probe current version")
  local expected_sources = variant_source_states[variant]
  t.eq(#probe.from_states, #expected_sources, name .. ": probe source state count")
  for index, expected in ipairs(expected_sources) do
    t.eq(probe.from_states[index], expected, name .. ": probe source state " .. tostring(index))
  end
  t.eq(probe.to_state, "blocked", name .. ": probe target state")
  t.eq(probe.incoming_version, tostring(fixture.incoming_version), name .. ": probe incoming version")
  t.eq(probe.target_version, nil, name .. ": probe target version")
end

local function assert_catalog_matches_observed_admission(fixture)
  local variant = fixture.variant or REVIEW_REJECT_VARIANT
  local event = fix_reconcile_event(fixture.incoming_version, variant)
  mock_current_pr(event, fixture)

  local result, probes, decisions, boundary_calls = observe_department(function()
    return run_real_department(event)
  end)

  local admission_phase = #probes == 0 and "pre-cas" or "cas"
  t.eq(admission_phase, fixture.admission_phase or "cas", fixture.name .. ": admission phase")
  t.eq(#probes, admission_phase == "cas" and 1 or 0, fixture.name .. ": real department CAS probe count")
  t.eq(#decisions, 1, fixture.name .. ": structured CAS decision count")
  local decision = decisions[1]
  t.eq(decision.dept, "reconcile", fixture.name .. ": CAS decision department")
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
  end

  local probe = probes[1]
  if probe ~= nil then
    assert_probe_shape(fixture.name, probe, variant, fixture)
    local observed = observed_admission(probe, boundary_reached)
    local evidence = evidence_from_probe(probe, variant)
    t.eq(evidence.current, probe.current, fixture.name .. ": catalog current comes from probe")
    t.eq(evidence.incoming_version, probe.incoming_version, fixture.name .. ": catalog incoming version comes from probe")
    t.eq(evidence.target_version, probe.target_version, fixture.name .. ": catalog target version comes from probe")
    t.eq(evidence.overlay_version, probe.incoming_version, fixture.name .. ": catalog overlay comes from probe")
    local actual = catalog.resolve(POLICY_ID, evidence)
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

local function assert_malformed_fails_closed(payload)
  h.mock_bot_env()
  local result, probes, decisions, boundary_calls = observe_department(function()
    return run_real_department(payload)
  end)
  t.eq(result.exit_code, 0, "malformed: production rejects without a pipeline error")
  t.eq(#probes, 0, "malformed: production rejects before CAS")
  t.eq(#boundary_calls, 0, "malformed: production rejects before admission boundary")
  t.eq(#decisions, 1, "malformed: structured rejection decision count")
  t.eq(decisions[1].outcome, "skip-foreign(proposal_id)", "malformed: legacy log outcome")
end

return {
  test_pr_fix_reconcile_review_reject_source_equal_applies_at_effect_boundary = function()
    assert_catalog_matches_observed_admission({
      name = "review-reject-source-equal",
      current_state = "reviewing",
      current_version = V_EQUAL,
      incoming_version = V_EQUAL,
      boundary_reached = true,
      admission_status = "apply",
      effect_count = 2,
      post_admission_disposition = "effect-emitted(blocked)",
      legacy_log_outcome = "applied",
    })
  end,

  test_pr_fix_reconcile_bounded_fix_source_equal_applies_at_effect_boundary = function()
    assert_catalog_matches_observed_admission({
      name = "bounded-fix-source-equal",
      variant = BOUNDED_FIX_VARIANT,
      current_state = "fixing",
      current_version = V_EQUAL,
      incoming_version = V_EQUAL,
      boundary_reached = true,
      admission_status = "apply",
      effect_count = 2,
      post_admission_disposition = "effect-emitted(blocked)",
      legacy_log_outcome = "applied",
    })
  end,

  test_pr_fix_reconcile_source_older_is_stale = function()
    assert_catalog_matches_observed_admission({
      name = "review-reject-source-older",
      current_state = "reviewing",
      current_version = V_EQUAL,
      incoming_version = V_OLDER,
      probe_outcome = "stale",
      admission_status = "stale",
      admission_reason_code = "incoming-version-older",
      legacy_log_outcome = "skip-stale(version-mismatch)",
    })
  end,

  test_pr_fix_reconcile_newer_predecessor_is_pending = function()
    assert_catalog_matches_observed_admission({
      name = "review-reject-newer-predecessor",
      current_state = "pr-open",
      current_version = V_EQUAL,
      incoming_version = V_NEWER,
      probe_outcome = "pending",
      admission_status = "pending",
      admission_reason_code = "source-marker-not-visible",
      expected_exit_code = 1,
      legacy_log_outcome = "retry-pending(from-state marker not yet visible)",
    })
  end,

  test_pr_fix_reconcile_unrelated_current_is_stale = function()
    assert_catalog_matches_observed_admission({
      name = "review-reject-unrelated-current",
      current_state = "awaiting-pr",
      current_version = V_EQUAL,
      incoming_version = V_EQUAL,
      probe_outcome = "stale",
      admission_status = "stale",
      admission_reason_code = "advanced-or-diverged",
      legacy_log_outcome = "skip-stale(version-mismatch)",
    })
  end,

  test_pr_fix_reconcile_ordering_equal_safe_different_is_stale_version_mismatch = function()
    t.is_true(
      V_ORDERING_EQUAL_CURRENT ~= V_ORDERING_EQUAL_INCOMING,
      "ordering-equal-safe-different: fixture versions must be byte-different"
    )
    t.is_true(
      transition_version.safe_version_segment(V_ORDERING_EQUAL_CURRENT)
        ~= transition_version.safe_version_segment(V_ORDERING_EQUAL_INCOMING),
      "ordering-equal-safe-different: safe segments must differ"
    )
    assert_catalog_matches_observed_admission({
      name = "review-reject-ordering-equal-safe-different",
      current_state = "reviewing",
      current_version = V_ORDERING_EQUAL_CURRENT,
      incoming_version = V_ORDERING_EQUAL_INCOMING,
      probe_outcome = "apply",
      admission_status = "stale",
      admission_reason_code = "version-mismatch",
      legacy_log_outcome = "skip-stale(version-mismatch)",
    })
  end,

  test_pr_fix_reconcile_raw_different_safe_equal_applies = function()
    t.is_true(
      V_SAFE_EQUIVALENT_CURRENT ~= V_SAFE_EQUIVALENT_INCOMING,
      "raw-different-safe-equal: fixture versions must be byte-different"
    )
    t.eq(
      transition_version.safe_version_segment(V_SAFE_EQUIVALENT_CURRENT),
      transition_version.safe_version_segment(V_SAFE_EQUIVALENT_INCOMING),
      "raw-different-safe-equal: fixture safe segments"
    )
    assert_catalog_matches_observed_admission({
      name = "review-reject-raw-different-safe-equal",
      current_state = "reviewing",
      current_version = V_SAFE_EQUIVALENT_CURRENT,
      incoming_version = V_SAFE_EQUIVALENT_INCOMING,
      boundary_reached = true,
      probe_outcome = "apply",
      admission_status = "apply",
      effect_count = 2,
      post_admission_disposition = "effect-emitted(blocked)",
      legacy_log_outcome = "applied",
    })
  end,

  test_pr_fix_reconcile_target_state_is_pre_cas_idempotent = function()
    assert_catalog_matches_observed_admission({
      name = "review-reject-target-state",
      current_state = "blocked",
      current_version = V_EQUAL,
      incoming_version = V_EQUAL,
      admission_phase = "pre-cas",
      legacy_log_outcome = "skip-idempotent(already terminal)",
    })
  end,

  test_pr_fix_reconcile_malformed_version_fails_closed_before_cas = function()
    local payload = fix_reconcile_event(V_EQUAL, REVIEW_REJECT_VARIANT)
    payload.issue_version = 42
    assert_malformed_fails_closed(payload)
  end,
}
