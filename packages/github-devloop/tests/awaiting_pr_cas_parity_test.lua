-- Non-circularity contract: production truth comes from the real observe_issue
-- department's named CAS probe and first post-CAS admission boundary. Catalog
-- evidence is copied from the observed probe arguments, never reconstructed
-- from fixture fields. Effects and legacy CAS logs remain separate axes.

local catalog = require("devloop.restart_cas_catalog")
local owner_pending_projection = require("devloop.restart_owner_pending_projection")
local inventories = {
  canonicalization = require("core.restart.canonicalization_inventory"),
  entry = require("core.restart.entry_inventory"),
  operator_reentry = require("core.restart.operator_reentry_inventory"),
}
local devloop_logging = require("devloop.logging")
local devloop_state = require("devloop.state")
local entity_lib = require("devloop.entity")
local entity_mocks = require("tests.entity_read_mock_helpers")
local h = require("tests.devloop_helpers")
local m_builders = require("devloop.markers.builders")
local t = h.t
local core = h.core
local projection = owner_pending_projection.derive(core.restart_package_name, core.restart_transition_table(), inventories)
local observe_issue_department = require("departments.observe_issue.main")

local POLICY_ID = "cas.legacy_awaiting_pr_v1"
local REPO = "owner/repo"
local ISSUE_NUMBER = 42
local PR_NUMBER = 7
local PROPOSAL_ID = "github-devloop/issue/owner/repo/42"
local PR_PROPOSAL_ID = "github-devloop/pr/owner/repo/7"
local V_OLDER = "ready/consensus-github-devloop/issue/owner/repo/42/2026-06-02T01-02-03Z"
local V_EQUAL = "ready/consensus-github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z"
local DELEGATION = "g1"
local BRANCH = "dev"
local HEAD_SHA = "0123456789abcdef0123456789abcdef01234567"
local MERGE_COMMIT_SHA = "1111111111111111111111111111111111111111"

local variants = {
  ["implementing\0awaiting-pr"] = "implementing_to_awaiting_pr",
  ["awaiting-pr\0merged"] = "awaiting_pr_to_merged",
  ["awaiting-pr\0ready"] = "awaiting_pr_to_ready",
  ["awaiting-pr\0blocked"] = "awaiting_pr_to_blocked",
}

local function probe_variant(from_states, to_state)
  if type(from_states) ~= "table" or #from_states ~= 1 then
    return nil
  end
  return variants[tostring(from_states[1]) .. "\0" .. tostring(to_state)]
end

local function observe_department(run)
  local probes = {}
  local decisions = {}
  local boundary_calls = {}
  local boundary_seen = {}
  local original_versioned = devloop_state.versioned_transition_status
  local original_log_cas = devloop_logging.log_cas_decision
  local original_parse_pr_proposal_id = entity_lib.parse_pr_proposal_id
  local original_has_state_marker = devloop_state.has_state_marker

  devloop_state.versioned_transition_status = function(
    current,
    from_states,
    to_state,
    incoming_version,
    target_version
  )
    local outcome = original_versioned(current, from_states, to_state, incoming_version, target_version)
    local variant = probe_variant(from_states, to_state)
    if variant ~= nil then
      table.insert(probes, {
        current = current,
        from_states = from_states,
        to_state = to_state,
        incoming_version = incoming_version,
        target_version = target_version,
        outcome = outcome,
        variant = variant,
      })
    end
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
  entity_lib.parse_pr_proposal_id = function(value)
    local probe = probes[#probes]
    if probe ~= nil
      and probe.variant == "implementing_to_awaiting_pr"
      and not boundary_seen[#probes] then
      boundary_seen[#probes] = true
      table.insert(boundary_calls, {
        kind = "delegated-child-identity",
        probe = probe,
        value = value,
      })
    end
    return original_parse_pr_proposal_id(value)
  end
  devloop_state.has_state_marker = function(comments, proposal_id, state, version)
    local probe = probes[#probes]
    if probe ~= nil
      and probe.variant ~= "implementing_to_awaiting_pr"
      and not boundary_seen[#probes] then
      boundary_seen[#probes] = true
      table.insert(boundary_calls, {
        kind = "exact-target-marker",
        probe = probe,
        proposal_id = proposal_id,
        state = state,
        version = version,
      })
    end
    return original_has_state_marker(comments, proposal_id, state, version)
  end

  local ok, result = pcall(run)
  devloop_state.has_state_marker = original_has_state_marker
  entity_lib.parse_pr_proposal_id = original_parse_pr_proposal_id
  devloop_logging.log_cas_decision = original_log_cas
  devloop_state.versioned_transition_status = original_versioned
  if not ok then
    error(result, 0)
  end
  return result, probes, decisions, boundary_calls
end

local function evidence_from_probe(probe)
  local definition = catalog.definition(POLICY_ID)
  local variant = definition and definition.variants[probe.variant]
  t.is_true(variant ~= nil, "observed awaiting-pr probe must select a catalog variant")
  t.eq(#probe.from_states, #variant.source_states, "catalog source-state count comes from probe signature")
  for index, source_state in ipairs(probe.from_states) do
    t.eq(variant.source_states[index], source_state, "catalog source state comes from probe signature")
  end
  t.eq(variant.target_state, probe.to_state, "catalog target state comes from probe signature")
  return {
    current = probe.current,
    variant = probe.variant,
    incoming_version = probe.incoming_version,
    target_version = probe.target_version,
  }
end

local function comment(body, created_at)
  return {
    id = tostring(created_at or body):gsub("[^%w_%-]", "_"):sub(1, 60),
    body = body,
    author_login = core._test_bot_login,
    created_at = created_at or "2026-06-03T01:00:00Z",
  }
end

local function parent_comments(fixture)
  local comments = {}
  if fixture.current_state ~= nil then
    table.insert(comments, comment(
      core.state_marker(PROPOSAL_ID, fixture.current_state, fixture.current_version),
      "2026-06-03T01:02:03Z"
    ))
  end
  if fixture.delegation ~= false then
    table.insert(comments, comment(m_builders.pr_delegation_marker(
      PROPOSAL_ID,
      fixture.pr_proposal_id or PR_PROPOSAL_ID,
      fixture.pr_number or PR_NUMBER,
      fixture.delegation_version or fixture.current_version,
      DELEGATION
    ), "2026-06-03T01:03:03Z"))
  end
  return comments
end

local function child_comments(fixture)
  local version = fixture.child_version or fixture.current_version
  local body = m_builders.pr_origin_marker(
    PROPOSAL_ID,
    ISSUE_NUMBER,
    "devloop-owner-repo-42-01HY",
    version,
    BRANCH
  )
  if fixture.child_state ~= nil then
    body = body .. "\n" .. core.state_marker(PROPOSAL_ID, fixture.child_state, version)
  end
  if fixture.child_state == "merged" then
    body = body .. "\n" .. m_builders.merged_marker(core, PROPOSAL_ID, PR_NUMBER, version, HEAD_SHA)
  end
  return { comment(body, "2026-06-03T01:04:03Z") }
end

local function mock_env()
  h.mock_bot_env()
  h.mock_write_env("")
  t.mock_command("gh api graphql", {
    stdout = '{"data":{"repository":{"issue":{"blockedBy":{"nodes":[]}}}}}\n',
    stderr = "",
    exit_code = 0,
  })
  t.mock_command('printf %s "$FKST_DEVLOOP_UPSTREAM_BRANCH"', {
    stdout = BRANCH,
    stderr = "",
    exit_code = 0,
  })
  t.mock_command('printf %s "$FKST_DEVLOOP_INTEGRATION_BRANCH"', {
    stdout = BRANCH,
    stderr = "",
    exit_code = 0,
  })
end

local function mock_reads(fixture, issue_comments, pr_comments)
  entity_mocks.mock_issue_view_selector(t, {
    repo = REPO,
    number = ISSUE_NUMBER,
    labels = { "fkst-dev:enabled", "fkst-dev:" .. tostring(fixture.current_state or "thinking") },
    comments = issue_comments,
    assignees = { core._test_bot_login },
    author_login = core._test_bot_login,
  }, "title,body,comments,labels,state,createdAt,updatedAt,assignees,author")
  entity_mocks.mock_pr_view_selector(t, {
    repo = REPO,
    number = PR_NUMBER,
    comments = pr_comments,
    head = "devloop-owner-repo-42-01HY",
    head_sha = HEAD_SHA,
    merge_commit_sha = MERGE_COMMIT_SHA,
    state = fixture.pr_state or "OPEN",
    base_branch = BRANCH,
    labels = {},
  }, entity_mocks.pr_origin_selector, fixture.pr_view_times)
end

local function run_real_department(fixture)
  local raises = {}
  local original_raise = raise
  raise = function(queue, payload)
    table.insert(raises, { queue = queue, payload = payload })
  end
  local event = {
    schema = "github-proxy.v1",
    type = "issue",
    repo = REPO,
    number = ISSUE_NUMBER,
    title = "Await delegated PR",
    state = "OPEN",
    updated_at = "2026-06-03T01:02:03Z",
    labels = { "fkst-dev:enabled", "fkst-dev:" .. tostring(fixture.current_state or "thinking") },
    dedup_key = fixture.raw_incoming_version or "owner/repo#issue#42@2026-06-03T01:02:03Z",
    source_ref = entity_lib.issue_source_ref(REPO, ISSUE_NUMBER),
  }
  local ok, failure = pcall(observe_issue_department.pipeline, {
    queue = "github-proxy.github_entity_changed",
    payload = event,
  })
  raise = original_raise
  return {
    exit_code = ok and 0 or 1,
    error = ok and nil or tostring(failure),
    raises = raises,
  }
end

local function emitted_state(result)
  for _, raised in ipairs(result.raises or {}) do
    if raised.queue == "github-proxy.github_issue_comment_request" then
      local state = tostring(raised.payload and raised.payload.body or ""):match('state="([^"]+)"')
      if state ~= nil then
        return state
      end
    end
  end
  return nil
end

local function observed_admission(probe, boundary_reached)
  if boundary_reached then
    t.eq(probe.outcome, "apply", "awaiting-pr boundary reach requires an applying CAS probe")
    return { status = "apply", reason_code = "apply" }
  end
  if probe.outcome == "pending" then
    return { status = "pending", reason_code = "source-marker-not-visible" }
  end
  if probe.outcome == "idempotent" then
    return { status = "idempotent", reason_code = "already-at-target" }
  end
  if probe.outcome == "stale" then
    if type(probe.current) == "table"
      and probe.current.version ~= nil
      and probe.incoming_version ~= nil
      and tostring(probe.current.version) ~= tostring(probe.incoming_version) then
      return { status = "stale", reason_code = "incoming-version-older" }
    end
    return { status = "stale", reason_code = "advanced-or-diverged" }
  end
  error("awaiting-pr CAS probe applied without reaching its admission boundary")
end

local function post_admission_disposition(result, boundary_reached)
  if not boundary_reached then
    return "not-admitted"
  end
  local state = emitted_state(result)
  if state ~= nil then
    return "effect-emitted(" .. state .. ")"
  end
  return "admitted-no-effect"
end

local function decision_for_probe(decisions, probe)
  for index = #decisions, 1, -1 do
    local decision = decisions[index]
    if decision.from_state == probe.from_states[1] and decision.to_state == probe.to_state then
      return decision
    end
  end
  return nil
end

local function assert_case(fixture)
  mock_env()
  local issue_comments = parent_comments(fixture)
  local pr_comments = child_comments(fixture)
  mock_reads(fixture, issue_comments, pr_comments)

  local result, probes, decisions, boundary_calls = observe_department(function()
    return run_real_department(fixture)
  end)

  local admission_phase = #probes == 0 and "pre-cas" or "cas"
  t.eq(admission_phase, fixture.admission_phase or "cas", fixture.name .. ": admission phase")
  t.eq(#probes, admission_phase == "cas" and 1 or 0, fixture.name .. ": real department CAS probe count")
  t.eq(#boundary_calls, fixture.boundary_reached and 1 or 0, fixture.name .. ": admission boundary reach")

  if admission_phase == "cas" then
    local probe = probes[1]
    local boundary_reached = #boundary_calls == 1
    t.eq(probe.current.state, fixture.current_state, fixture.name .. ": probe current state")
    t.eq(probe.current.version, fixture.current_version, fixture.name .. ": probe current version")
    t.eq(probe.incoming_version, fixture.current_version, fixture.name .. ": production feeds current marker version to CAS")
    t.eq(probe.target_version, nil, fixture.name .. ": production CAS target version")
    if fixture.raw_incoming_version ~= nil then
      t.is_true(
        tostring(probe.incoming_version) ~= tostring(fixture.raw_incoming_version),
        fixture.name .. ": raw fixture input is not production CAS evidence"
      )
    end

    local evidence = evidence_from_probe(probe)
    t.eq(evidence.current, probe.current, fixture.name .. ": catalog current comes from probe")
    t.eq(evidence.incoming_version, probe.incoming_version, fixture.name .. ": catalog incoming version comes from probe")
    t.eq(evidence.target_version, probe.target_version, fixture.name .. ": catalog target version comes from probe")
    local observed = observed_admission(probe, boundary_reached)
    local actual = catalog.resolve(POLICY_ID, evidence, projection)
    t.eq(actual.status, observed.status, fixture.name .. ": admission status parity")
    t.eq(actual.reason_code, observed.reason_code, fixture.name .. ": admission reason parity")
    if fixture.admission_status ~= nil then
      t.eq(observed.status, fixture.admission_status, fixture.name .. ": observed admission status")
      t.eq(actual.status, fixture.admission_status, fixture.name .. ": catalog admission status")
    end

    local decision = decision_for_probe(decisions, probe)
    t.is_true(decision ~= nil, fixture.name .. ": legacy CAS decision captured separately")
    if fixture.legacy_log_outcome ~= nil then
      t.eq(decision.outcome, fixture.legacy_log_outcome, fixture.name .. ": legacy log outcome")
    end
  else
    t.eq(#boundary_calls, 0, fixture.name .. ": pre-CAS input cannot reach admission boundary")
  end

  t.eq(
    result.exit_code,
    fixture.expected_exit_code or 0,
    fixture.name .. ": department exit code error=" .. tostring(result.error)
  )
  t.eq(#result.raises, fixture.effect_count or 0, fixture.name .. ": captured effect count")
  t.eq(
    post_admission_disposition(result, #boundary_calls == 1),
    fixture.post_admission_disposition or "not-admitted",
    fixture.name .. ": post-admission disposition"
  )
end

return {
  test_implementing_canonicalization_uses_spied_current_version_not_raw_fixture_version = function()
    assert_case({
      name = "implementing-canonicalization-derived-version",
      current_state = "implementing",
      current_version = V_EQUAL,
      raw_incoming_version = V_OLDER,
      child_state = "merged",
      pr_state = "MERGED",
      boundary_reached = true,
      admission_status = "apply",
      effect_count = 2,
      post_admission_disposition = "effect-emitted(awaiting-pr)",
      legacy_log_outcome = "applied(merged-delegated-pr-canonicalized)",
    })
  end,

  test_awaiting_pr_merged_uses_spied_probe_evidence = function()
    assert_case({
      name = "awaiting-pr-to-merged",
      current_state = "awaiting-pr",
      current_version = V_EQUAL,
      child_state = "merged",
      pr_state = "MERGED",
      boundary_reached = true,
      admission_status = "apply",
      effect_count = 2,
      post_admission_disposition = "effect-emitted(merged)",
      legacy_log_outcome = "applied(child-pr-merged)",
    })
  end,

  test_awaiting_pr_closed_child_uses_spied_probe_evidence = function()
    assert_case({
      name = "awaiting-pr-to-ready",
      current_state = "awaiting-pr",
      current_version = V_EQUAL,
      child_state = "closed-unmerged",
      boundary_reached = true,
      admission_status = "apply",
      effect_count = 2,
      post_admission_disposition = "effect-emitted(ready)",
      legacy_log_outcome = "applied(child-pr-closed-unmerged)",
    })
  end,

  test_awaiting_pr_blocked_child_uses_spied_probe_evidence = function()
    assert_case({
      name = "awaiting-pr-to-blocked",
      current_state = "awaiting-pr",
      current_version = V_EQUAL,
      child_state = "blocked",
      boundary_reached = true,
      admission_status = "apply",
      effect_count = 2,
      post_admission_disposition = "effect-emitted(blocked)",
      legacy_log_outcome = "applied(child-pr-blocked)",
    })
  end,

  test_child_lineage_mismatch_is_classified_pre_cas = function()
    assert_case({
      name = "awaiting-pr-child-lineage-mismatch",
      current_state = "awaiting-pr",
      current_version = V_EQUAL,
      child_state = "blocked",
      child_version = V_OLDER,
      admission_phase = "pre-cas",
    })
  end,

  test_nonterminal_child_is_classified_pre_cas = function()
    assert_case({
      name = "awaiting-pr-child-nonterminal",
      current_state = "awaiting-pr",
      current_version = V_EQUAL,
      child_state = "reviewing",
      admission_phase = "pre-cas",
    })
  end,

  test_managed_target_marker_is_classified_pre_cas = function()
    assert_case({
      name = "awaiting-pr-managed-target-marker",
      current_state = "ready",
      current_version = V_EQUAL .. "/reimplement/1",
      delegation_version = V_EQUAL,
      child_state = "closed-unmerged",
      child_version = V_EQUAL,
      admission_phase = "pre-cas",
    })
  end,

  test_wrong_canonicalization_source_is_classified_pre_cas = function()
    assert_case({
      name = "canonicalization-wrong-source",
      current_state = "ready",
      current_version = V_EQUAL .. "/reimplement/1",
      delegation_version = V_EQUAL,
      child_state = "merged",
      child_version = V_EQUAL,
      pr_state = "MERGED",
      admission_phase = "pre-cas",
    })
  end,
}
