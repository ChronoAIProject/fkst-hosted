-- Non-circularity contract: production truth comes from the real review_meta
-- department's CAS probe and the result-marker check admission boundary. Effects
-- and legacy CAS logs are recorded as separate post-admission observations. This
-- test never computes the expected result with a devloop.state transition helper.

local catalog = require("devloop.restart_cas_catalog")
local devloop_logging = require("devloop.logging")
local m_builders = require("devloop.markers.builders")
local m_facts = require("devloop.markers.facts")
local devloop_state = require("devloop.state")
local dispatch_live_run = require("devloop.dispatch_live_run")
local h = require("tests.devloop_helpers")
local t = h.t
local core = h.core
local review_meta_department = require("departments.review_meta.main")

local POLICY_ID = "cas.legacy_review_meta_v1"
local VARIANT = "predecision_eligibility"
local V_OLDER = "ready/consensus-github-devloop/issue/owner/repo/42/2026-06-02T01-02-03Z"
local V_EQUAL = "ready/consensus-github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z"
local V_NEWER = "ready/consensus-github-devloop/issue/owner/repo/42/2026-06-04T01-02-03Z"
local V_ORDERING_EQUAL_CURRENT = V_EQUAL .. "/loop/01"
local V_ORDERING_EQUAL_INCOMING = V_EQUAL .. "/loop/1"

local function mock_meta_codex(stdout)
  t.mock_command('printf %s "$FKST_RUNTIME_ROOT"', {
    stdout = "/tmp/fkst-packages-test/github-devloop/runtime",
    stderr = "",
    exit_code = 0,
  })
  t.mock_command("mkdir -p", { stdout = "", stderr = "", exit_code = 0 })
  t.mock_command("codex exec", { stdout = stdout, stderr = "", exit_code = 0 })
end

local function observe_department(run)
  local probes = {}
  local decisions = {}
  local marker_checks = {}
  local original_cyclic = devloop_state.cyclic_transition_status
  local original_log_cas = devloop_logging.log_cas_decision
  local original_has_review_meta_marker = m_facts.has_review_meta_marker
  -- A fresh fixture has no live codex run; force the liveness dedup to its true
  -- no-live-run value so the post-admission effect path is deterministic. The
  -- live-run registry is keyed by (role, proposal_id, dedup_key) and is shared
  -- across cases, so leaving it real makes emit-vs-defer depend on test ordering
  -- (a cross-case lease artifact), not on the CAS admission this harness verifies.
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
  m_facts.has_review_meta_marker = function(comments, proposal_id, dedup_key)
    table.insert(marker_checks, {
      comments = comments,
      proposal_id = proposal_id,
      dedup_key = dedup_key,
    })
    return original_has_review_meta_marker(comments, proposal_id, dedup_key)
  end

  local ok, result = pcall(run)
  dispatch_live_run.dispatch_live_run_dedup = original_dispatch_live_run_dedup
  m_facts.has_review_meta_marker = original_has_review_meta_marker
  devloop_logging.log_cas_decision = original_log_cas
  devloop_state.cyclic_transition_status = original_cyclic
  if not ok then
    error(result, 0)
  end
  return result, probes, decisions, marker_checks
end

local function current_fact(state, version)
  return { state = state, version = version }
end

local function evidence_from_fixture(fixture)
  return {
    current = current_fact(fixture.current_state, fixture.current_version),
    variant = VARIANT,
    incoming_version = fixture.incoming_version,
    overlay_version = fixture.incoming_version,
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

local function observed_admission(probe, decision, marker_check_reached)
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
    error("review-meta admission probe returned an unknown outcome: " .. tostring(probe.outcome))
  end

  local legacy_reason = tostring(decision and decision.reason or "")
  if not marker_check_reached and legacy_reason:find("no longer review%-meta") ~= nil then
    return { status = "stale", reason_code = "from-state-mismatch" }
  end
  if not marker_check_reached and legacy_reason:find("version") ~= nil then
    return { status = "stale", reason_code = "version-mismatch" }
  end
  if marker_check_reached then
    return { status = "apply", reason_code = "apply" }
  end
  error("review-meta admission apply did not reach a classified guard")
end

local function post_admission_disposition(result, decision, marker_check_reached)
  if not marker_check_reached then
    return "not-admitted"
  end
  local state = emitted_state(result)
  if state ~= nil then
    return "effect-emitted(" .. state .. ")"
  end
  local outcome = tostring(decision and decision.outcome or "")
  if outcome:find("review%-meta marker already visible") ~= nil then
    return "effect-idempotent"
  end
  if outcome:find("live%-exec%-ref") ~= nil then
    return "liveness-deferred"
  end
  return "codex-defer"
end

local function run_real_department(event)
  local raises = {}
  local original_raise = raise
  raise = function(queue, payload)
    table.insert(raises, { queue = queue, payload = payload })
  end
  local ok, failure = pcall(review_meta_department.pipeline, {
    queue = "devloop_review_meta",
    payload = event,
  })
  raise = original_raise
  return {
    exit_code = ok and 0 or 1,
    error = ok and nil or tostring(failure),
    raises = raises,
  }
end

local function assert_catalog_matches_observed_decision(fixture)
  local event = h.review_meta_event({ version = fixture.incoming_version })
  local comments = {}
  if fixture.current_state ~= nil then
    table.insert(comments, core.state_marker(event.proposal_id, fixture.current_state, fixture.current_version))
  end
  if fixture.result_marker_visible then
    table.insert(comments, m_builders.review_meta_marker(
      event.proposal_id,
      event.dedup_key,
      "block",
      core.next_review_meta_action_version(event.version)
    ))
  end
  h.mock_issue_review_meta({}, comments)
  if fixture.codex_stdout ~= nil then
    mock_meta_codex(fixture.codex_stdout)
    h.mock_context_bundle(event)
  end
  h.mock_default_issue_claim()
  h.mock_pr_origin(nil, "devloop-owner-repo-42-01HY", "def456", "OPEN", "dev")

  local result, probes, decisions, marker_checks = observe_department(function()
    return run_real_department(event)
  end)

  t.eq(#probes, 1, fixture.name .. ": real department CAS probe count")
  local probe = probes[1]
  t.eq(probe.current.state, fixture.current_state, fixture.name .. ": probe current state")
  t.eq(probe.current.version, fixture.current_version, fixture.name .. ": probe current version")
  t.eq(probe.from_states[1], "review-meta", fixture.name .. ": probe source state")
  t.eq(#probe.from_states, 1, fixture.name .. ": probe source state count")
  t.eq(probe.to_state, "fixing", fixture.name .. ": legacy shared probe target")
  t.eq(probe.incoming_version, fixture.incoming_version, fixture.name .. ": probe incoming version")
  t.eq(probe.target_version, nil, fixture.name .. ": probe target version")

  t.eq(#decisions, 1, fixture.name .. ": structured CAS decision count")
  local decision = decisions[1]
  t.eq(decision.dept, "review_meta", fixture.name .. ": CAS decision department")
  t.eq(decision.from_state, "review-meta", fixture.name .. ": logged source state")
  t.eq(decision.to_state, "fixing|blocked", fixture.name .. ": logged decision family")
  t.is_true(type(decision.outcome) == "string", fixture.name .. ": legacy log outcome captured")
  t.is_true(type(decision.reason) == "string", fixture.name .. ": legacy log reason captured")

  local marker_check_reached = #marker_checks > 0
  t.eq(#marker_checks, fixture.marker_check_reached and 1 or 0, fixture.name .. ": marker admission boundary reach")
  if marker_check_reached then
    t.eq(marker_checks[1].proposal_id, event.proposal_id, fixture.name .. ": marker boundary proposal")
    t.eq(marker_checks[1].dedup_key, event.dedup_key, fixture.name .. ": marker boundary dedup")
  end
  local observed = observed_admission(probe, decision, marker_check_reached)
  local actual = catalog.resolve(POLICY_ID, evidence_from_fixture(fixture))
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

  local disposition = post_admission_disposition(result, decision, marker_check_reached)
  t.eq(disposition, fixture.post_admission_disposition or "not-admitted", fixture.name .. ": post-admission disposition")
  if fixture.legacy_log_outcome ~= nil then
    -- The from-state skip currently logs "applied". That production logging bug
    -- belongs in a separate R9 PR; Step 0 records and reconciles around it.
    t.eq(decision.outcome, fixture.legacy_log_outcome, fixture.name .. ": legacy log outcome")
  end

  if fixture.effect_state ~= nil then
    t.eq(probe.outcome, "apply", fixture.name .. ": decision reached only after the shared probe applied")
    t.eq(emitted_state(result), fixture.effect_state, fixture.name .. ": emitted effect target")
  else
    t.eq(emitted_state(result), nil, fixture.name .. ": non-apply case emitted no state effect")
  end
end

local function assert_rejected_before_cas(name, payload, expected_reason_code)
  local _, probes = observe_department(function()
    return run_real_department(payload)
  end)
  t.eq(#probes, 0, name .. ": invalid production input must not reach CAS")
  local evidence = {
    current = current_fact("review-meta", V_EQUAL),
    variant = VARIANT,
    incoming_version = payload.version,
    overlay_version = payload.version,
  }
  t.eq(evidence.incoming_version, payload.version, name .. ": catalog incoming version comes from rejected payload")
  t.eq(evidence.overlay_version, payload.version, name .. ": catalog overlay version comes from rejected payload")
  local resolved = catalog.resolve(POLICY_ID, evidence)
  t.eq(resolved.status, "illegal", name .. ": catalog status")
  t.eq(resolved.reason_code, expected_reason_code, name .. ": catalog reason")
  t.eq(resolved.cas_outcome, "illegal(" .. expected_reason_code .. ")", name .. ": catalog fails closed")
end

return {
  test_review_meta_source_older_is_stale = function()
    assert_catalog_matches_observed_decision({
      name = "review-meta-source-older",
      current_state = "review-meta",
      current_version = V_EQUAL,
      incoming_version = V_OLDER,
    })
  end,

  test_review_meta_source_equal_fix_applies_to_fixing = function()
    assert_catalog_matches_observed_decision({
      name = "review-meta-source-equal-fix",
      current_state = "review-meta",
      current_version = V_EQUAL,
      incoming_version = V_EQUAL,
      codex_stdout = h.action_label .. " fix\n" .. h.reason_label .. " Run another fix pass.\nBlocking gap: missing CAS parity guard",
      effect_state = "fixing",
      marker_check_reached = true,
      post_admission_disposition = "effect-emitted(fixing)",
    })
  end,

  test_review_meta_source_equal_block_applies_to_blocked = function()
    assert_catalog_matches_observed_decision({
      name = "review-meta-source-equal-block",
      current_state = "review-meta",
      current_version = V_EQUAL,
      incoming_version = V_EQUAL,
      codex_stdout = h.action_label .. " block\n" .. h.reason_label .. " The review cannot be repaired safely.",
      effect_state = "blocked",
      marker_check_reached = true,
      post_admission_disposition = "effect-emitted(blocked)",
    })
  end,

  test_review_meta_ordering_equal_raw_different_is_stale_version_mismatch = function()
    t.is_true(
      V_ORDERING_EQUAL_CURRENT ~= V_ORDERING_EQUAL_INCOMING,
      "review-meta-ordering-equal-raw-different: fixture versions must be byte-different"
    )
    assert_catalog_matches_observed_decision({
      name = "review-meta-ordering-equal-raw-different",
      current_state = "review-meta",
      current_version = V_ORDERING_EQUAL_CURRENT,
      incoming_version = V_ORDERING_EQUAL_INCOMING,
      probe_outcome = "apply",
      admission_status = "stale",
      admission_reason_code = "version-mismatch",
    })
  end,

  test_review_meta_source_equal_visible_result_marker_is_admitted_then_idempotent = function()
    assert_catalog_matches_observed_decision({
      name = "review-meta-source-equal-result-visible",
      current_state = "review-meta",
      current_version = V_EQUAL,
      incoming_version = V_EQUAL,
      result_marker_visible = true,
      marker_check_reached = true,
      post_admission_disposition = "effect-idempotent",
    })
  end,

  test_review_meta_source_newer_is_pending = function()
    assert_catalog_matches_observed_decision({
      name = "review-meta-source-newer",
      current_state = "review-meta",
      current_version = V_EQUAL,
      incoming_version = V_NEWER,
      expected_exit_code = 1,
    })
  end,

  test_review_meta_missing_current_marker_is_pending = function()
    assert_catalog_matches_observed_decision({
      name = "review-meta-current-missing",
      current_state = nil,
      current_version = nil,
      incoming_version = V_EQUAL,
      expected_exit_code = 1,
    })
  end,

  test_review_meta_target_state_is_idempotent = function()
    assert_catalog_matches_observed_decision({
      name = "review-meta-target-idempotent",
      current_state = "fixing",
      current_version = V_EQUAL,
      incoming_version = V_EQUAL,
    })
  end,

  test_review_meta_unrelated_state_is_stale = function()
    assert_catalog_matches_observed_decision({
      name = "review-meta-unrelated-stale",
      current_state = "blocked",
      current_version = V_EQUAL,
      incoming_version = V_EQUAL,
    })
  end,

  test_review_meta_predecessor_with_older_event_is_stale = function()
    assert_catalog_matches_observed_decision({
      name = "review-meta-predecessor-older",
      current_state = "reviewing",
      current_version = V_EQUAL,
      incoming_version = V_OLDER,
    })
  end,

  test_review_meta_predecessor_with_equal_event_matches_production_admission = function()
    assert_catalog_matches_observed_decision({
      name = "review-meta-predecessor-equal",
      current_state = "reviewing",
      current_version = V_EQUAL,
      incoming_version = V_EQUAL,
      legacy_log_outcome = "applied",
    })
  end,

  test_review_meta_malformed_evidence_and_payload_fail_closed_before_cas = function()
    local payload = h.review_meta_event()
    payload.version = 42
    assert_rejected_before_cas("review-meta-malformed-version", payload, "invalid-evidence")
  end,
}
