local devloop_state = require("devloop.state")
local entity_read_mocks = require("tests.entity_read_mock_helpers")
local h = require("tests.devloop_helpers")
local m_builders = require("devloop.markers.builders")
local observation_support = require("tests.old_behavior_observation_support_helpers")
local testing = require("testkit.testing")
local transition_version = require("contract.transition_version")
local observe_pr_department = require("departments.observe_pr.main")

local t = h.t
local core = h.core
local JSON_NULL = observation_support.JSON_NULL
local canonical_json = observation_support.canonical_json
local copy_value = observation_support.copy_value
local first_difference = observation_support.first_difference
local json_array = observation_support.json_array
local INVENTORY_PATH = "migration/restart-lifecycle.inventory.json"
local SITE = {
  path = "packages/github-devloop-pr/departments/observe_pr/main.lua",
  symbol = "process_pr_event",
  ordinal = "versioned_transition_status:pr-open|unmanaged->reviewing",
}

local REPO = "owner/repo"
local ISSUE_NUMBER = 42
local PROPOSAL_ID = "github-devloop/issue/owner/repo/42"
local BRANCH = "devloop-owner-repo-42-01HY"
local BASE_BRANCH = "dev"
local HEAD_SHA = "def456"
local V_OLDER = "ready/consensus-github-devloop/issue/owner/repo/42/2026-06-02T01-02-03Z"
local V_EQUAL = "ready/consensus-github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z"
local V_NEWER = "ready/consensus-github-devloop/issue/owner/repo/42/2026-06-04T01-02-03Z"
local V_ORDERING_EQUAL_CURRENT = V_EQUAL .. "/loop/01"
local V_ORDERING_EQUAL_INCOMING = V_EQUAL .. "/loop/1"

local function pr_event(fixture)
  local pr_number = fixture.pr_number
  return {
    queue = "github-proxy.github_entity_changed",
    payload = {
      schema = "github-proxy.v1",
      type = "pr",
      repo = REPO,
      number = pr_number,
      state = "OPEN",
      updated_at = "2026-06-04T01:02:03Z",
      dedup_key = "owner/repo#pr#" .. tostring(pr_number) .. "@2026-06-04T01:02:03Z",
      source_ref = {
        kind = "external",
        ref = "owner/repo#pr/" .. tostring(pr_number),
      },
    },
    now_seconds = 1784048400,
  }
end

local function fixture_comments(fixture)
  local comments = json_array({
    m_builders.pr_origin_marker(
      PROPOSAL_ID,
      tostring(ISSUE_NUMBER),
      BRANCH,
      fixture.incoming_version,
      BASE_BRANCH
    ),
  })
  if fixture.current_state ~= nil then
    table.insert(comments, core.state_marker(PROPOSAL_ID, fixture.current_state, fixture.current_version))
  end
  return comments
end

local function prepare_fixture(fixture)
  h.mock_bot_env()
  entity_read_mocks.mock_pr_read_forms(t, {
    repo = REPO,
    number = fixture.pr_number,
    comments = fixture_comments(fixture),
    head = BRANCH,
    head_sha = fixture.missing_head_sha and "" or HEAD_SHA,
    state = fixture.pr_state or "OPEN",
    base_branch = BASE_BRANCH,
    labels = {},
    times = 2,
  })
  entity_read_mocks.mock_issue_view_selector(t, {
    repo = REPO,
    number = ISSUE_NUMBER,
    assignees = { "fkst-test-bot" },
    author_login = "fkst-test-bot",
  }, "assignees,author", 1)
end

local function observe_real_department(run)
  return observation_support.observe_department({
    dept = "observe_pr",
    from_state = "pr-open",
    transition_kind = "versioned_transition_status",
    run = run,
    codex_runs_for_read = json_array(),
    write_mode = "real",
  })
end

local EFFECTS = {
  ["github-devloop.reviewing"] = {
    effect_id = "comment:pr:observe-pr-reviewing",
    queue = "github-proxy.github_pr_comment_request",
    sink_kind = "comment",
    authority_class = "lifecycle-authoritative",
  },
  ["github-devloop.closed_unmerged"] = {
    effect_id = "comment:pr:observe-pr-closed-unmerged",
    queue = "github-proxy.github_pr_comment_request",
    sink_kind = "comment",
    authority_class = "lifecycle-authoritative",
  },
  ["github-proxy.github_issue_comment_request"] = {
    effect_id = "comment:issue:observe-pr-merged-replay",
    queue = "github-proxy.github_issue_comment_request",
    sink_kind = "comment",
    authority_class = "lifecycle-authoritative",
  },
  ["github-proxy.github_issue_label_request"] = {
    effect_id = "label:issue:observe-pr-merged-replay",
    queue = "github-proxy.github_issue_label_request",
    sink_kind = "label",
    authority_class = "lifecycle-authoritative",
  },
}

local function outcome_status(probe, decision, apply)
  if decision == nil then
    error("OLD observe_pr probe has no routed decision: " .. canonical_json(probe))
  end
  if probe.outcome == "stale" then
    t.eq(decision.outcome, "skip-stale(version-mismatch)", "older input reaches the raw-version guard")
    t.eq(apply, nil, "older input emits no apply effect")
    return "stale", "incoming-version-older", decision.outcome
  end
  if probe.outcome ~= "apply" then
    error("unreachable OLD observe_pr CAS regime reached: " .. tostring(probe.outcome))
  end
  if decision.outcome == "skip-stale(version-mismatch)" then
    t.eq(apply, nil, "raw-version mismatch emits no apply effect")
    return "stale", "version-mismatch", decision.outcome
  end
  if decision.outcome == "skip-stale(pr-closed)" then
    t.eq(apply, nil, "closed unmanaged ingress emits no apply effect")
    return "apply", "pr-closed-no-effect", decision.outcome
  end
  if decision.outcome == "applied(orphaned-pr-closed)" then
    t.eq(apply.to_state, "closed-unmerged", "closed pr-open replay applies the terminal child state")
    return "apply", "closed-pr-replay", decision.outcome
  end
  if decision.outcome == "skip-foreign(head)" then
    t.eq(apply, nil, "merged replay without a head sha emits no apply effect")
    return "skip-foreign(head)", "merged-pr-missing-head", decision.outcome
  end
  if decision.outcome == "applied(linked-pr-merged)" then
    t.eq(apply.to_state, "merged", "merged pr-open replay applies the terminal issue state")
    return "apply", "merged-pr-replay", decision.outcome
  end
  if decision.outcome == "applied" then
    t.eq(apply.to_state, "reviewing", "open admitted input applies reviewing")
    return "apply", "apply", decision.outcome
  end
  error("unclassified OLD observe_pr outcome: decision=" .. tostring(decision.outcome)
    .. " probe=" .. tostring(probe.outcome))
end

local function effects_from_raises(raises)
  local effects = json_array()
  local writes = json_array()
  for ordinal, raised in ipairs(raises) do
    local handoff_kind = raised.payload and raised.payload.handoff and raised.payload.handoff.kind
    local shape = EFFECTS[handoff_kind] or EFFECTS[raised.queue]
    if shape == nil then
      error("unclassified OLD observe_pr raise handoff: " .. tostring(handoff_kind))
    end
    t.eq(raised.queue, shape.queue, "observe_pr OLD writer queue")
    table.insert(effects, {
      effect_id = shape.effect_id,
      sink_kind = shape.sink_kind,
      authority_class = shape.authority_class,
      ordinal = ordinal,
    })
    table.insert(writes, {
      effect_id = shape.effect_id,
      queue = raised.queue,
      payload = copy_value(raised.payload),
    })
  end
  return effects, writes
end

local function build_record(fixture, event, result, captured)
  return observation_support.build_record({
    t = t,
    dept = "observe_pr",
    event = event,
    result = result,
    captured = captured,
    owner = "github-devloop-pr",
    site = SITE,
    observation_prefix = "writer:github-devloop-pr:observe-pr-reviewing",
    observation_variant = fixture.name,
    transition_kind = "versioned_transition_status",
    source_state = function(probe)
      return probe.current.state
    end,
    source_boundary = function(probe, runtime_event)
      if probe.current.state == nil then
        return runtime_event.queue
      end
      return nil
    end,
    outcome_status = outcome_status,
    effects_from_raises = effects_from_raises,
    lineage = function(payload, probe, decision)
      return {
        proposal_id = decision.proposal_id,
        pr_number = payload.number,
        dedup_key = payload.dedup_key,
        incoming_version = probe.incoming_version,
        source_ref = copy_value(payload.source_ref),
      }
    end,
  })
end

local FIXTURES = {
  {
    name = "pr-open-apply",
    pr_number = 9701,
    current_state = "pr-open",
    current_version = V_EQUAL,
    incoming_version = V_EQUAL,
    expected_probe = "apply",
    expected_status = "apply",
    expected_reason = "apply",
    expected_cas_outcome = "applied",
    expected_effects = 1,
    expected_source_state = "pr-open",
    expected_source_boundary = JSON_NULL,
  },
  {
    name = "unmanaged-ingress-apply",
    pr_number = 9702,
    incoming_version = V_EQUAL,
    expected_probe = "apply",
    expected_status = "apply",
    expected_reason = "apply",
    expected_cas_outcome = "applied",
    expected_effects = 1,
    expected_source_state = JSON_NULL,
    expected_source_boundary = "github-proxy.github_entity_changed",
  },
  {
    name = "incoming-older",
    pr_number = 9703,
    current_state = "pr-open",
    current_version = V_EQUAL,
    incoming_version = V_OLDER,
    expected_probe = "stale",
    expected_status = "stale",
    expected_reason = "incoming-version-older",
    expected_cas_outcome = "skip-stale(version-mismatch)",
    expected_effects = 0,
    expected_source_state = "pr-open",
    expected_source_boundary = JSON_NULL,
  },
  {
    name = "incoming-newer-raw-mismatch",
    pr_number = 9704,
    current_state = "pr-open",
    current_version = V_EQUAL,
    incoming_version = V_NEWER,
    expected_probe = "apply",
    expected_status = "stale",
    expected_reason = "version-mismatch",
    expected_cas_outcome = "skip-stale(version-mismatch)",
    expected_effects = 0,
    expected_source_state = "pr-open",
    expected_source_boundary = JSON_NULL,
  },
  {
    name = "ordering-equal-raw-mismatch",
    pr_number = 9705,
    current_state = "pr-open",
    current_version = V_ORDERING_EQUAL_CURRENT,
    incoming_version = V_ORDERING_EQUAL_INCOMING,
    expected_probe = "apply",
    expected_status = "stale",
    expected_reason = "version-mismatch",
    expected_cas_outcome = "skip-stale(version-mismatch)",
    expected_effects = 0,
    expected_source_state = "pr-open",
    expected_source_boundary = JSON_NULL,
  },
  {
    name = "pr-open-closed-replay",
    pr_number = 9706,
    current_state = "pr-open",
    current_version = V_EQUAL,
    incoming_version = V_EQUAL,
    pr_state = "CLOSED",
    expected_probe = "apply",
    expected_status = "apply",
    expected_reason = "closed-pr-replay",
    expected_cas_outcome = "applied(orphaned-pr-closed)",
    expected_effects = 1,
    expected_source_state = "pr-open",
    expected_source_boundary = JSON_NULL,
  },
  {
    name = "unmanaged-ingress-closed",
    pr_number = 9707,
    incoming_version = V_EQUAL,
    pr_state = "CLOSED",
    expected_probe = "apply",
    expected_status = "apply",
    expected_reason = "pr-closed-no-effect",
    expected_cas_outcome = "skip-stale(pr-closed)",
    expected_effects = 0,
    expected_source_state = JSON_NULL,
    expected_source_boundary = "github-proxy.github_entity_changed",
  },
  {
    name = "pr-open-merged-missing-head-sha",
    pr_number = 9709,
    current_state = "pr-open",
    current_version = V_EQUAL,
    incoming_version = V_EQUAL,
    pr_state = "MERGED",
    missing_head_sha = true,
    expected_probe = "apply",
    expected_status = "skip-foreign(head)",
    expected_reason = "merged-pr-missing-head",
    expected_cas_outcome = "skip-foreign(head)",
    expected_effects = 0,
    expected_source_state = "pr-open",
    expected_source_boundary = JSON_NULL,
  },
  {
    name = "pr-open-merged-replay",
    pr_number = 9708,
    current_state = "pr-open",
    current_version = V_EQUAL,
    incoming_version = V_EQUAL,
    pr_state = "MERGED",
    expected_probe = "apply",
    expected_status = "apply",
    expected_reason = "merged-pr-replay",
    expected_cas_outcome = "applied(linked-pr-merged)",
    expected_effects = 2,
    expected_source_state = "pr-open",
    expected_source_boundary = JSON_NULL,
  },
}

local function capture_fixture(fixture)
  local event = pr_event(fixture)
  prepare_fixture(fixture)
  local result, captured = observe_real_department(function()
    return testing.run_fake(observe_pr_department, event)
  end)
  if #captured.probes ~= 1 then
    error(fixture.name .. ": expected one versioned probe; lines=" .. canonical_json(captured.lines))
  end
  local record = build_record(fixture, event, result, captured)
  t.eq(captured.probes[1].outcome, fixture.expected_probe, fixture.name .. ": real CAS regime")
  t.eq(record.old_outcome.status, fixture.expected_status, fixture.name .. ": captured status")
  t.eq(record.old_outcome.reason_code, fixture.expected_reason, fixture.name .. ": captured disposition")
  t.eq(record.old_outcome.cas_outcome, fixture.expected_cas_outcome, fixture.name .. ": routed CAS outcome")
  t.eq(#record.old_outcome.emitted_effects, fixture.expected_effects, fixture.name .. ": runtime effect count")
  t.eq(record.typed_intent.source_state, fixture.expected_source_state, fixture.name .. ": typed source state")
  t.eq(record.typed_intent.source_boundary, fixture.expected_source_boundary, fixture.name .. ": typed source boundary")
  t.eq(record.old_inputs.target_version, JSON_NULL, fixture.name .. ": four-argument versioned target_version")
  t.eq(captured.liveness_read_count, 0, fixture.name .. ": ingress observer never reads Codex liveness")
  return record
end

local function is_target_record(record)
  local site = type(record) == "table" and record.site or nil
  return type(site) == "table"
    and site.path == SITE.path
    and site.symbol == SITE.symbol
    and site.ordinal == SITE.ordinal
end

local function committed_records()
  local inventory = json.decode(file.read(INVENTORY_PATH))
  local selected = json_array()
  for _, record in ipairs(inventory.old_behavior_observations or {}) do
    if is_target_record(record) then
      table.insert(selected, record)
    end
  end
  table.sort(selected, function(left, right)
    return tostring(left.observation_id) < tostring(right.observation_id)
  end)
  return selected
end

return {
  test_observe_pr_old_observations_are_runtime_bound_to_inventory = function()
    t.is_true(V_ORDERING_EQUAL_CURRENT ~= V_ORDERING_EQUAL_INCOMING, "raw versions are byte-distinct")
    t.eq(
      transition_version.compare(V_ORDERING_EQUAL_CURRENT, V_ORDERING_EQUAL_INCOMING),
      0,
      "raw-version mismatch includes the ordering-equal regime"
    )

    local actual = json_array()
    for _, fixture in ipairs(FIXTURES) do
      local ok, record = pcall(capture_fixture, fixture)
      if not ok then
        error("OLD observe_pr fixture " .. fixture.name .. " failed: " .. tostring(record), 0)
      end
      table.insert(actual, record)
    end
    table.sort(actual, function(left, right)
      return left.observation_id < right.observation_id
    end)

    local expected = committed_records()
    local difference = first_difference(actual, expected, "old_behavior_observations[observe_pr]")
    if difference ~= nil or canonical_json(actual) ~= canonical_json(expected) then
      error(
        "runtime-bound OLD observe_pr observation differs at " .. tostring(difference or "canonical-json")
          .. "; runtime_records=" .. canonical_json(actual),
        0
      )
    end
  end,
}
