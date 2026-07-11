-- Non-circularity contract: production truth comes from the real review_loop
-- department's local safe-segment CAS probe and its first downstream effect-
-- idempotency guard. Catalog evidence is built only from the observed probe
-- arguments; no transition helper computes the expected admission result.

local catalog = require("devloop.restart_cas_catalog")
local convergence_shared = require("devloop.convergence.shared")
local conv_rounds = require("devloop.convergence.rounds")
local devloop_base = require("devloop.base")
local entity_lib = require("devloop.entity")
local devloop_logging = require("devloop.logging")
local m_builders = require("devloop.markers.builders")
local transition_version = require("contract.transition_version")
local h = require("tests.devloop_helpers")
local t = h.t
local core = h.core
local review_loop_department = require("departments.review_loop.main")

local POLICY_ID = "cas.legacy_review_loop_safe_v1"
local PROPOSAL_ID = "github-devloop/issue/owner/repo/42"
local REPO = "owner/repo"
local PR_NUMBER = 7
local BRANCH = "devloop-owner-repo-42-01HY"
local BASE_BRANCH = "dev"
local HEAD_SHA = "def456"
local V_OLDER = "ready/consensus-github-devloop/issue/owner/repo/42/2026-06-02T01-02-03Z"
local V_EQUAL = "ready/consensus-github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z"
local V_NEWER = "ready/consensus-github-devloop/issue/owner/repo/42/2026-06-04T01-02-03Z"

local V_LONG = V_EQUAL
for _ = 1, 6 do
  V_LONG = core.next_fix_version(V_LONG)
end

local function observe_department(run)
  local probes = {}
  local decisions = {}
  local boundary_calls = {}
  local parsed_review_versions = {}
  local original_parse_pr_review_proposal_id = devloop_base.parse_pr_review_proposal_id
  local original_current_entity_state = entity_lib.current_entity_state
  local original_log_cas = devloop_logging.log_cas_decision
  local original_has_review_converge_round_marker = conv_rounds.has_review_converge_round_marker

  -- The production probe is local and the test runtime intentionally omits the
  -- debug library. Observe its exact two inputs at their adjacent production
  -- providers: parse_pr_review_proposal_id supplies review_version, then
  -- current_entity_state supplies current immediately before the probe call.
  -- INVARIANT (keep true or this pairing drifts): the probe's parse is the LAST
  -- parse before its current_entity_state — validation parses the proposal id
  -- earlier, so parsed_review_versions[#...] at the current_entity_state call is
  -- the probe's review_version. If a future edit inserts another parse between the
  -- probe's parse and its current_entity_state, capture the probe's parse by
  -- proposal identity instead of taking the latest.
  devloop_base.parse_pr_review_proposal_id = function(...)
    local parsed = { original_parse_pr_review_proposal_id(...) }
    table.insert(parsed_review_versions, parsed[3])
    return table.unpack(parsed)
  end
  entity_lib.current_entity_state = function(...)
    local current = original_current_entity_state(...)
    table.insert(probes, {
      current = current,
      review_version = parsed_review_versions[#parsed_review_versions],
    })
    return current
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
      probe_count = #probes,
    })
    return original_log_cas(dept, proposal_id, current, from_state, to_state, outcome, reason)
  end
  conv_rounds.has_review_converge_round_marker = function(...)
    local args = { ... }
    local visible = original_has_review_converge_round_marker(...)
    table.insert(boundary_calls, {
      review_proposal_id = args[3],
      issue_proposal_id = args[4],
      issue_version = args[5],
      head_sha = args[6],
      round = args[8],
      visible = visible,
      probe_count = #probes,
    })
    return visible
  end

  local ok, result = pcall(run)
  conv_rounds.has_review_converge_round_marker = original_has_review_converge_round_marker
  devloop_logging.log_cas_decision = original_log_cas
  entity_lib.current_entity_state = original_current_entity_state
  devloop_base.parse_pr_review_proposal_id = original_parse_pr_review_proposal_id
  if not ok then
    error(result, 0)
  end
  if #probes == 1 then
    if #boundary_calls > 0 then
      probes[1].outcome = "apply"
    elseif #decisions == 1 and tostring(decisions[1].outcome):find("skip-stale(reviewing-version)", 1, true) ~= nil then
      probes[1].outcome = "stale"
    elseif #decisions == 1 and tostring(decisions[1].reason):find("marker not yet visible", 1, true) ~= nil then
      probes[1].outcome = "pending"
    end
  end
  return result, probes, decisions, boundary_calls
end

local function evidence_from_probe(probe)
  return {
    current = probe.current,
    review_version = probe.review_version,
  }
end

local function review_event(version, overrides)
  local proposal_id = devloop_base.pr_review_proposal_id(REPO, PR_NUMBER, version, HEAD_SHA)
  local event = h.review_unresolved({
    proposal_id = proposal_id,
    dedup_key = "consensus:" .. proposal_id .. "/review",
  })
  for key, value in pairs(overrides or {}) do
    event[key] = value
  end
  return event
end

local function visible_round_marker(event, issue_version)
  return conv_rounds.review_converge_round_marker(
    core,
    event.proposal_id,
    PROPOSAL_ID,
    issue_version,
    HEAD_SHA,
    convergence_shared.source_ref_digest(event.source_ref),
    0,
    event.dedup_key,
    event.narrowed_question,
    event.angle_digests,
    event.findings_record,
    event.essence_stall == true
  )
end

local function run_real_department(event)
  local raises = {}
  local original_raise = raise
  raise = function(queue, payload)
    table.insert(raises, { queue = queue, payload = payload })
  end
  local ok, failure = pcall(review_loop_department.pipeline, {
    queue = "consensus.consensus_converge",
    payload = event,
  })
  raise = original_raise
  return {
    exit_code = ok and 0 or 1,
    error = ok and nil or tostring(failure),
    raises = raises,
  }
end

local function observed_admission(probe, boundary_reached)
  if probe == nil then
    return { status = "pre-cas", reason_code = "cas-probe-not-reached" }
  end
  if probe.outcome == "pending" then
    return { status = "pending", reason_code = "source-marker-not-visible" }
  end
  if probe.outcome == "stale" then
    return { status = "stale", reason_code = "reviewing-version" }
  end
  if probe.outcome ~= "apply" then
    error("review-loop CAS probe returned an unknown outcome: " .. tostring(probe.outcome))
  end
  if boundary_reached then
    return { status = "apply", reason_code = "apply" }
  end
  error("review-loop CAS apply did not reach the admission boundary")
end

local function post_admission_disposition(result, decision, boundary_reached)
  if not boundary_reached then
    return "not-admitted"
  end
  if #(result.raises or {}) > 0 then
    return "effect-emitted"
  end
  if tostring(decision and decision.outcome or ""):find("round marker already visible", 1, true) ~= nil then
    return "effect-idempotent"
  end
  return "admitted-no-effect"
end

local function assert_review_loop_admission_case(fixture)
  local event = review_event(fixture.event_version)
  local comments = {
    m_builders.pr_origin_marker(PROPOSAL_ID, "42", BRANCH, fixture.event_version, BASE_BRANCH),
  }
  if fixture.current_state ~= nil then
    table.insert(comments, core.state_marker(PROPOSAL_ID, fixture.current_state, fixture.current_version))
  end
  if fixture.result_marker_visible then
    table.insert(comments, visible_round_marker(event, fixture.current_version))
  end

  h.mock_bot_env()
  h.mock_default_issue_claim(REPO, 42)
  h.mock_pr_origin_for({
    number = PR_NUMBER,
    comments = comments,
    head = BRANCH,
    head_sha = HEAD_SHA,
    state = "OPEN",
    base_branch = BASE_BRANCH,
  })

  local result, probes, decisions, boundary_calls = observe_department(function()
    return run_real_department(event)
  end)

  t.eq(result.exit_code, fixture.expected_exit_code or 0, fixture.name .. ": department exit code")
  t.eq(#probes, 1, fixture.name .. ": real department CAS probe count")
  local probe = probes[1]
  t.eq(probe.current.state, fixture.current_state, fixture.name .. ": probe current state")
  t.eq(probe.current.version, fixture.current_version, fixture.name .. ": probe current version")
  t.is_true(type(probe.review_version) == "string", fixture.name .. ": probe review version")

  t.eq(#decisions, 1, fixture.name .. ": structured CAS decision count")
  local decision = decisions[1]
  t.eq(decision.dept, "review_loop", fixture.name .. ": CAS decision department")
  t.eq(decision.from_state, "reviewing", fixture.name .. ": logged source state")
  t.is_true(type(decision.outcome) == "string", fixture.name .. ": legacy log outcome captured")
  t.is_true(type(decision.reason) == "string", fixture.name .. ": legacy log reason captured")

  local boundary_reached = #boundary_calls > 0
  t.eq(boundary_reached, fixture.boundary_reached == true, fixture.name .. ": admission boundary reach")
  if boundary_reached then
    t.eq(#boundary_calls, 1, fixture.name .. ": admission boundary call count")
    t.eq(boundary_calls[1].probe_count, 1, fixture.name .. ": admission boundary follows probe")
    t.eq(boundary_calls[1].review_proposal_id, event.proposal_id, fixture.name .. ": boundary review proposal")
    t.eq(boundary_calls[1].issue_proposal_id, PROPOSAL_ID, fixture.name .. ": boundary issue proposal")
    t.eq(boundary_calls[1].issue_version, fixture.current_version, fixture.name .. ": boundary issue version")
    t.eq(boundary_calls[1].visible, fixture.result_marker_visible == true, fixture.name .. ": boundary marker visibility")
  end

  local observed = observed_admission(probe, boundary_reached)
  local resolved = catalog.resolve(POLICY_ID, evidence_from_probe(probe))
  t.eq(resolved.status, observed.status, fixture.name .. ": admission status parity")
  t.eq(resolved.reason_code, observed.reason_code, fixture.name .. ": admission reason parity")
  t.eq(probe.outcome, fixture.probe_outcome, fixture.name .. ": literal probe outcome")
  t.eq(observed.status, fixture.admission_status, fixture.name .. ": observed admission status")
  t.eq(resolved.status, fixture.admission_status, fixture.name .. ": catalog admission status")
  t.eq(
    post_admission_disposition(result, decision, boundary_reached),
    fixture.post_admission_disposition or "not-admitted",
    fixture.name .. ": post-admission disposition"
  )
  if fixture.legacy_log_outcome ~= nil then
    t.eq(decision.outcome, fixture.legacy_log_outcome, fixture.name .. ": legacy log outcome")
  end
  t.eq(#result.raises, 0, fixture.name .. ": captured effects")
end

local function assert_malformed_is_pre_cas_and_catalog_illegal()
  local malformed = review_event(V_EQUAL, { proposal_id = 42 })
  local result, probes, decisions, boundary_calls = observe_department(function()
    return run_real_department(malformed)
  end)

  t.eq(result.exit_code, 0, "review-loop-malformed: production rejects unsupported payload")
  t.eq(#probes, 0, "review-loop-malformed: production rejects before CAS")
  t.eq(#boundary_calls, 0, "review-loop-malformed: admission boundary is not reached")
  t.eq(#result.raises, 0, "review-loop-malformed: no effects")
  t.eq(#decisions, 1, "review-loop-malformed: rejection decision count")
  t.eq(decisions[1].outcome, "skip-foreign(proposal_id)", "review-loop-malformed: rejection outcome")

  local resolved = catalog.resolve(POLICY_ID, {
    current = { state = "reviewing", version = V_EQUAL },
    review_version = malformed.proposal_id,
  })
  t.eq(resolved.status, "illegal", "review-loop-malformed: catalog status")
  t.eq(resolved.reason_code, "invalid-evidence", "review-loop-malformed: catalog reason")
  t.eq(resolved.cas_outcome, "illegal(invalid-evidence)", "review-loop-malformed: catalog fails closed")
end

return {
  test_review_loop_source_and_target_equal_applies_before_effect_idempotency = function()
    assert_review_loop_admission_case({
      name = "review-loop-source-target-equal",
      current_state = "reviewing",
      current_version = V_EQUAL,
      event_version = V_EQUAL,
      result_marker_visible = true,
      boundary_reached = true,
      probe_outcome = "apply",
      admission_status = "apply",
      post_admission_disposition = "effect-idempotent",
      legacy_log_outcome = "skip-idempotent(review converge round marker already visible)",
    })
  end,

  test_review_loop_safe_equal_raw_different_applies = function()
    t.is_true(V_LONG ~= transition_version.safe_version_segment(V_LONG), "review-loop-safe-equal: raw fixture differs")
    assert_review_loop_admission_case({
      name = "review-loop-safe-equal",
      current_state = "reviewing",
      current_version = V_LONG,
      event_version = V_LONG,
      result_marker_visible = true,
      boundary_reached = true,
      probe_outcome = "apply",
      admission_status = "apply",
      post_admission_disposition = "effect-idempotent",
    })
  end,

  test_review_loop_older_review_version_is_stale = function()
    assert_review_loop_admission_case({
      name = "review-loop-older-stale",
      current_state = "reviewing",
      current_version = V_EQUAL,
      event_version = V_OLDER,
      probe_outcome = "stale",
      admission_status = "stale",
      legacy_log_outcome = "skip-stale(reviewing-version)",
    })
  end,

  test_review_loop_newer_review_waits_for_reviewing_source = function()
    assert_review_loop_admission_case({
      name = "review-loop-newer-pending",
      current_state = "pr-open",
      current_version = V_EQUAL,
      event_version = V_NEWER,
      probe_outcome = "pending",
      admission_status = "pending",
      expected_exit_code = 1,
    })
  end,

  test_review_loop_missing_source_marker_is_pending = function()
    assert_review_loop_admission_case({
      name = "review-loop-missing-pending",
      current_state = nil,
      current_version = nil,
      event_version = V_EQUAL,
      probe_outcome = "pending",
      admission_status = "pending",
      expected_exit_code = 1,
    })
  end,

  test_review_loop_unrelated_later_current_is_stale = function()
    assert_review_loop_admission_case({
      name = "review-loop-unrelated-stale",
      current_state = "fixing",
      current_version = V_EQUAL,
      event_version = V_EQUAL,
      probe_outcome = "stale",
      admission_status = "stale",
    })
  end,

  test_review_loop_same_state_safe_version_mismatch_is_stale = function()
    assert_review_loop_admission_case({
      name = "review-loop-safe-version-mismatch",
      current_state = "reviewing",
      current_version = V_EQUAL,
      event_version = V_NEWER,
      probe_outcome = "stale",
      admission_status = "stale",
    })
  end,

  test_review_loop_malformed_evidence_and_payload_fail_closed_before_cas = function()
    assert_malformed_is_pre_cas_and_catalog_illegal()
  end,
}
