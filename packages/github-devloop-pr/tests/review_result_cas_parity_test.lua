-- Non-circularity contract: production truth comes from the real review_result
-- department's CAS probe and the exact safe-version guard applied to those spied
-- arguments. The comment builder, effects, and legacy CAS logs are separate
-- post-admission observations. This test never computes the expected result with
-- a devloop.state transition helper.

local catalog = require("devloop.restart_cas_catalog")
local owner_pending_projection = require("devloop.restart_owner_pending_projection")
local inventories = {
  canonicalization = require("core.restart.canonicalization_inventory"),
  entry = require("core.restart.entry_inventory"),
  operator_reentry = require("core.restart.operator_reentry_inventory"),
}
local devloop_base = require("devloop.base")
local config = require("devloop.config")
local devloop_logging = require("devloop.logging")
local m_builders = require("devloop.markers.builders")
local requests_review = require("devloop.requests.review")
local devloop_state = require("devloop.state")
local transition_version = require("contract.transition_version")
local h = require("tests.devloop_helpers")
local t = h.t
local core = h.core
local projection = owner_pending_projection.derive(core.restart_package_name, core.restart_transition_table(), inventories)
local review_result_department = require("departments.review_result.main")

local POLICY_ID = "cas.legacy_review_result_v1"
local V_OLDER = "2026-06-02T01-02-03Z"
local V_EQUAL = "2026-06-03T01-02-03Z"
local V_NEWER = "2026-06-04T01-02-03Z"
local V_ORDERING_EQUAL_CURRENT = "v-loop-01"
local V_ORDERING_EQUAL_INCOMING = "v-loop-1"

local function version_at_fix_round(round)
  local version = V_EQUAL
  for _ = 1, round do
    version = core.next_fix_version(version)
  end
  return version
end

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
  local comment_builders = {}
  local original_cyclic = devloop_state.cyclic_transition_status
  local original_log_cas = devloop_logging.log_cas_decision
  local original_build_review_result = requests_review.build_review_result_comment_request

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
  requests_review.build_review_result_comment_request = function(
    package_core,
    repo,
    issue_number,
    issue_proposal_id,
    issue_version,
    reached,
    source_ref
  )
    table.insert(comment_builders, {
      repo = repo,
      issue_number = issue_number,
      issue_proposal_id = issue_proposal_id,
      issue_version = issue_version,
      reached = reached,
      source_ref = source_ref,
    })
    return original_build_review_result(
      package_core,
      repo,
      issue_number,
      issue_proposal_id,
      issue_version,
      reached,
      source_ref
    )
  end

  local ok, result = pcall(run)
  requests_review.build_review_result_comment_request = original_build_review_result
  devloop_logging.log_cas_decision = original_log_cas
  devloop_state.cyclic_transition_status = original_cyclic
  if not ok then
    error(result, 0)
  end
  return result, probes, decisions, comment_builders
end

local PROBE_VARIANTS = {
  ["reviewing->merge-ready"] = "reviewing_to_merge_ready",
  ["reviewing->fixing"] = "reviewing_to_fixing",
  ["reviewing->review-meta"] = "reviewing_to_review_meta",
}

local function evidence_from_probe(probe)
  local source_states = table.concat(probe.from_states, ",")
  local variant = PROBE_VARIANTS[source_states .. "->" .. tostring(probe.to_state)]
  if variant == nil then
    error("review-result CAS probe has no catalog variant: " .. source_states .. "->" .. tostring(probe.to_state))
  end
  return {
    current = probe.current,
    source_states = probe.from_states,
    to_state = probe.to_state,
    variant = variant,
    incoming_version = probe.incoming_version,
    target_version = probe.target_version,
    overlay_version = probe.incoming_version,
  }
end

local function review_event(fixture)
  local proposal_id = devloop_base.pr_review_proposal_id(
    "owner/repo",
    7,
    fixture.incoming_version,
    "def456"
  )
  local decision = fixture.decision or "reject"
  return h.review_reached({
    proposal_id = proposal_id,
    dedup_key = "consensus:" .. proposal_id .. "/review",
    decision = decision,
    body = decision == "approve"
      and "Review consensus approves the diff."
      or "Review consensus rejects the diff.",
    blocking_gap = decision == "reject" and "missing CAS admission parity guard" or nil,
  })
end

local function emitted_state(result)
  for _, raised in ipairs(result.raises or {}) do
    if raised.queue == "github-proxy.github_pr_comment_request" then
      return tostring(raised.payload and raised.payload.body or ""):match('state="([^"]+)"')
    end
  end
  return nil
end

local function observed_admission(probe)
  if probe.outcome == "pending" then
    return { status = "pending", reason_code = "source-marker-not-visible" }
  end
  if probe.outcome == "idempotent" then
    return { status = "idempotent", reason_code = "already-at-target" }
  end
  if probe.outcome == "stale" then
    if transition_version.compare(probe.incoming_version, probe.current.version) < 0 then
      return { status = "stale", reason_code = "incoming-version-older" }
    end
    return { status = "stale", reason_code = "advanced-or-diverged" }
  end
  if probe.outcome ~= "apply" then
    error("review-result admission probe returned an unknown outcome: " .. tostring(probe.outcome))
  end

  -- The probe receives current_review_version after production's safe projection.
  if tostring(probe.current.version) ~= tostring(probe.incoming_version) then
    return { status = "stale", reason_code = "version-mismatch" }
  end
  return { status = "apply", reason_code = "apply" }
end

local function post_admission_disposition(result, admitted, comment_builder_reached)
  if not admitted then
    return "not-admitted"
  end
  if not comment_builder_reached then
    return "comment-builder-not-reached"
  end
  local state = emitted_state(result)
  if state ~= nil then
    return "effect-emitted(" .. state .. ")"
  end
  return "effect-not-emitted"
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

local function prepare_valid_fixture(fixture, event)
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
  if event.decision == "approve" then
    h.mock_pr_normal_risk_diff_name_only()
  end
end

local function assert_catalog_matches_observed_decision(fixture)
  local event = review_event(fixture)
  prepare_valid_fixture(fixture, event)

  local result, probes, decisions, comment_builders = observe_department(function()
    return run_real_department(event)
  end)

  if #probes == 0 then
    t.eq(#comment_builders, 0, fixture.name .. ": pre-CAS input must not reach comment builder")
    t.eq(post_admission_disposition(result, false, false), "not-admitted", fixture.name .. ": pre-CAS disposition")
    t.eq(#result.raises, 0, fixture.name .. ": pre-CAS input must emit no effects")
    return "pre-cas"
  end

  t.eq(#probes, 1, fixture.name .. ": real department CAS probe count")
  local probe = probes[1]
  t.eq(probe.current.state, fixture.current_state, fixture.name .. ": probe current state")
  t.eq(probe.from_states[1], "reviewing", fixture.name .. ": probe source state")
  t.eq(#probe.from_states, 1, fixture.name .. ": probe source state count")
  if fixture.target_state ~= nil then
    t.eq(probe.to_state, fixture.target_state, fixture.name .. ": probe target state")
  end
  t.eq(probe.target_version, nil, fixture.name .. ": probe target version")

  t.eq(#decisions, 1, fixture.name .. ": structured CAS decision count")
  local decision = decisions[1]
  t.eq(decision.dept, "review_result", fixture.name .. ": CAS decision department")
  t.eq(decision.from_state, "reviewing", fixture.name .. ": logged source state")
  t.eq(decision.to_state, fixture.logged_target_state or probe.to_state, fixture.name .. ": logged target state")
  t.is_true(type(decision.outcome) == "string", fixture.name .. ": legacy log outcome captured")
  t.is_true(type(decision.reason) == "string", fixture.name .. ": legacy log reason captured")

  local comment_builder_reached = #comment_builders > 0
  t.eq(
    #comment_builders,
    fixture.comment_builder_reached and 1 or 0,
    fixture.name .. ": post-admission comment builder reach"
  )
  if comment_builder_reached then
    t.eq(probe.outcome, "apply", fixture.name .. ": comment builder requires an applied probe")
    local boundary = comment_builders[1]
    t.eq(boundary.repo, "owner/repo", fixture.name .. ": boundary repo")
    t.eq(boundary.issue_number, "42", fixture.name .. ": boundary issue")
    t.eq(
      boundary.issue_proposal_id,
      "github-devloop/issue/owner/repo/42",
      fixture.name .. ": boundary issue proposal"
    )
    t.eq(boundary.reached.proposal_id, event.proposal_id, fixture.name .. ": boundary review proposal")
  end

  local observed = observed_admission(probe)
  local disposition = post_admission_disposition(result, observed.status == "apply", comment_builder_reached)
  t.eq(disposition, fixture.post_admission_disposition or "not-admitted", fixture.name .. ": post-admission disposition")
  local expected_queues = fixture.expected_queues or {}
  t.eq(#result.raises, #expected_queues, fixture.name .. ": captured effect count")
  for index, expected_queue in ipairs(expected_queues) do
    t.eq(result.raises[index].queue, expected_queue, fixture.name .. ": captured effect queue " .. tostring(index))
  end
  if fixture.effect_state ~= nil then
    t.eq(probe.outcome, "apply", fixture.name .. ": effect follows an applied shared probe")
    t.eq(emitted_state(result), fixture.effect_state, fixture.name .. ": emitted effect target")
  else
    t.eq(emitted_state(result), nil, fixture.name .. ": non-apply case emitted no state effect")
  end

  local evidence = evidence_from_probe(probe)
  t.eq(evidence.current, probe.current, fixture.name .. ": catalog current comes from probe")
  t.eq(evidence.source_states, probe.from_states, fixture.name .. ": catalog source states come from probe")
  t.eq(evidence.to_state, probe.to_state, fixture.name .. ": catalog target state comes from probe")
  t.eq(evidence.incoming_version, probe.incoming_version, fixture.name .. ": catalog incoming version comes from probe")
  t.eq(evidence.target_version, probe.target_version, fixture.name .. ": catalog target version comes from probe")
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
  if fixture.legacy_log_outcome ~= nil then
    t.eq(decision.outcome, fixture.legacy_log_outcome, fixture.name .. ": legacy log outcome")
  end
  return "cas"
end

local function assert_rejected_before_cas(name, payload)
  local result, probes, _, comment_builders = observe_department(function()
    return run_real_department(payload)
  end)
  t.eq(#probes, 0, name .. ": invalid production input must not reach CAS")
  t.eq(#comment_builders, 0, name .. ": invalid production input must not reach comment builder")
  t.eq(result.exit_code, 0, name .. ": production rejects malformed input without failing the pipeline")
  t.eq(#result.raises, 0, name .. ": malformed production input must emit no effects")
  t.eq(post_admission_disposition(result, false, false), "not-admitted", name .. ": pre-CAS disposition")
end

return {
  test_review_result_source_older_is_stale = function()
    assert_catalog_matches_observed_decision({
      name = "review-result-source-older",
      current_state = "reviewing",
      current_version = V_EQUAL,
      incoming_version = V_OLDER,
      target_state = "fixing",
    })
  end,

  test_review_result_source_equal_reject_applies_to_fixing = function()
    assert_catalog_matches_observed_decision({
      name = "review-result-source-equal-reject",
      current_state = "reviewing",
      current_version = V_EQUAL,
      incoming_version = V_EQUAL,
      target_state = "fixing",
      comment_builder_reached = true,
      effect_state = "fixing",
      post_admission_disposition = "effect-emitted(fixing)",
      expected_queues = {
        "github-proxy.github_pr_comment_request",
        "github-proxy.github_issue_label_request",
      },
    })
  end,

  test_review_result_source_equal_approve_applies_to_merge_ready = function()
    assert_catalog_matches_observed_decision({
      name = "review-result-source-equal-approve",
      decision = "approve",
      current_state = "reviewing",
      current_version = V_EQUAL,
      incoming_version = V_EQUAL,
      target_state = "merge-ready",
      comment_builder_reached = true,
      effect_state = "merge-ready",
      post_admission_disposition = "effect-emitted(merge-ready)",
      expected_queues = {
        "github-proxy.github_pr_comment_request",
        "github-proxy.github_issue_label_request",
      },
    })
  end,

  test_review_result_source_equal_reflection_applies_to_review_meta = function()
    local reflection_review_version = core.next_fix_version(core.next_fix_version(V_EQUAL))
    assert_catalog_matches_observed_decision({
      name = "review-result-source-equal-reflection",
      target_state = "review-meta",
      current_state = "reviewing",
      current_version = reflection_review_version,
      incoming_version = reflection_review_version,
      comment_builder_reached = true,
      effect_state = "review-meta",
      post_admission_disposition = "effect-emitted(review-meta)",
      expected_queues = {
        "github-proxy.github_pr_comment_request",
        "github-proxy.github_issue_label_request",
        "devloop_review_meta",
      },
    })
  end,

  test_review_result_ordering_equal_safe_different_is_stale_version_mismatch = function()
    local current_safe = transition_version.safe_version_segment(V_ORDERING_EQUAL_CURRENT)
    local incoming_safe = transition_version.safe_version_segment(V_ORDERING_EQUAL_INCOMING)
    t.is_true(
      current_safe ~= incoming_safe,
      "review-result-ordering-equal-safe-different: fixture safe versions must be byte-different"
    )
    t.eq(
      transition_version.compare(current_safe, incoming_safe),
      0,
      "review-result-ordering-equal-safe-different: fixture versions must be ordering-equal"
    )
    assert_catalog_matches_observed_decision({
      name = "review-result-ordering-equal-safe-different",
      current_state = "reviewing",
      current_version = V_ORDERING_EQUAL_CURRENT,
      incoming_version = V_ORDERING_EQUAL_INCOMING,
      target_state = "fixing",
      probe_outcome = "apply",
      admission_status = "stale",
      admission_reason_code = "version-mismatch",
      legacy_log_outcome = "skip-stale(version-mismatch)",
    })
  end,

  test_review_result_source_newer_is_pending = function()
    assert_catalog_matches_observed_decision({
      name = "review-result-source-newer",
      current_state = "reviewing",
      current_version = V_EQUAL,
      incoming_version = V_NEWER,
      target_state = "fixing",
      expected_exit_code = 1,
    })
  end,

  test_review_result_missing_current_marker_is_pending = function()
    assert_catalog_matches_observed_decision({
      name = "review-result-current-missing",
      decision = "approve",
      current_state = nil,
      current_version = nil,
      incoming_version = V_EQUAL,
      target_state = "merge-ready",
      expected_exit_code = 1,
    })
  end,

  test_review_result_target_state_is_idempotent = function()
    assert_catalog_matches_observed_decision({
      name = "review-result-target-idempotent",
      current_state = "fixing",
      current_version = V_EQUAL,
      incoming_version = V_EQUAL,
      target_state = "fixing",
    })
  end,

  test_review_result_unrelated_state_is_stale = function()
    assert_catalog_matches_observed_decision({
      name = "review-result-unrelated-stale",
      current_state = "blocked",
      current_version = V_EQUAL,
      incoming_version = V_EQUAL,
      target_state = "fixing",
    })
  end,

  test_review_result_predecessor_equal_probe_apply_matches_from_state_overlay = function()
    assert_catalog_matches_observed_decision({
      name = "review-result-predecessor-equal-overlay",
      current_state = "pr-open",
      current_version = V_EQUAL,
      incoming_version = V_EQUAL,
      target_state = "fixing",
      probe_outcome = "apply",
      comment_builder_reached = true,
      effect_state = "fixing",
      post_admission_disposition = "effect-emitted(fixing)",
      expected_queues = {
        "github-proxy.github_pr_comment_request",
        "github-proxy.github_issue_label_request",
      },
    })
  end,

  test_review_result_capped_reject_is_admitted_before_comment_builder = function()
    local capped_version = version_at_fix_round(config.max_fix_rounds())
    assert_catalog_matches_observed_decision({
      name = "review-result-capped-reject",
      current_state = "reviewing",
      current_version = capped_version,
      incoming_version = capped_version,
      target_state = "fixing",
      logged_target_state = "blocked",
      probe_outcome = "apply",
      admission_status = "apply",
      admission_reason_code = "apply",
      post_admission_disposition = "comment-builder-not-reached",
      expected_queues = {
        "devloop_fix_reconcile",
        "github-devloop-decompose.devloop_decompose",
      },
      legacy_log_outcome = "applied(fix-loop-max-rounds)",
    })
  end,

  test_review_result_malformed_evidence_and_payload_fail_closed_before_cas = function()
    local payload = h.review_reached()
    payload.proposal_id = 42
    payload.dedup_key = 42
    assert_rejected_before_cas("review-result-malformed-version-source", payload)
  end,
}
