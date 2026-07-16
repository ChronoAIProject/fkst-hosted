local config = require("devloop.config")
local conv_reconcile = require("devloop.convergence.reconcile")
local devloop_base = require("devloop.base")
local devloop_logging = require("devloop.logging")
local devloop_state = require("devloop.state")
local entity_read_mocks = require("tests.entity_read_mock_helpers")
local h = require("tests.devloop_helpers")
local m_builders = require("devloop.markers.builders")
local observation_support = require("testkit.old_behavior_observation_support")
local payloads_builders = require("devloop.payloads.builders")
local testing = require("testkit.testing")
local observe_issue_department = require("departments.observe_issue.main")

local t = h.t
local core = h.core
local JSON_NULL = observation_support.JSON_NULL
local canonical_json = observation_support.canonical_json
local copy_value = observation_support.copy_value
local first_difference = observation_support.first_difference
local json_array = observation_support.json_array
local nullable = observation_support.nullable
local INVENTORY_PATH = "migration/restart-lifecycle.inventory.json"
local SITE = {
  path = "packages/github-devloop/departments/observe_issue/main.lua",
  symbol = "process_issue_event",
  ordinal = "build_devloop_ready_payload",
}

local REPO = "owner/repo"
local ISSUE_NUMBER = 42
local PROPOSAL_ID = "github-devloop/issue/owner/repo/42"
local SOURCE_REF = {
  kind = "external",
  ref = "owner/repo#issue/42",
}
local INNER_VERSION = "consensus-github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z"
local READY_VERSION = "ready/" .. INNER_VERSION
local BLOCKED_REVIEW_VERSION = READY_VERSION .. "/review-loop/3"
local BLOCKED_TIMEOUT_VERSION = conv_reconcile.timeout_reconcile_state_version(
  READY_VERSION,
  "implementing",
  3
)

local function trusted_comment(body, id, created_at)
  return {
    id = id,
    body = body,
    author_login = "fkst-test-bot",
    created_at = created_at or "2026-06-03T01:00:00Z",
  }
end

local function trusted_reimplement_command(id)
  return trusted_comment("fkst: reimplement", id, "2026-06-03T02:00:00Z")
end

local FIXTURES = {
  {
    name = "impl-failed",
    state = "impl-failed",
    state_version = READY_VERSION,
    labels = { "fkst-dev:enabled", "fkst-dev:impl-failed" },
    comments = function()
      return json_array({
        trusted_comment(core.state_marker(PROPOSAL_ID, "impl-failed", READY_VERSION), "IC_state_impl_failed"),
        trusted_comment(core.impl_failure_marker(PROPOSAL_ID, READY_VERSION, "codex-failed", 2), "IC_impl_failure"),
        trusted_reimplement_command("IC_reimplement_impl_failed"),
      })
    end,
    reason_code = "operator-reimplement-impl-failed",
  },
  {
    name = "blocked-open-pr",
    state = "blocked",
    state_version = BLOCKED_REVIEW_VERSION,
    labels = { "fkst-dev:enabled", "fkst-dev:blocked" },
    comments = function()
      return json_array({
        trusted_comment(m_builders.pr_link_marker(
          PROPOSAL_ID,
          7,
          "devloop-owner-repo-42-01HY",
          READY_VERSION,
          "dev"
        ), "IC_pr_link"),
        trusted_comment(core.state_marker(PROPOSAL_ID, "blocked", BLOCKED_REVIEW_VERSION), "IC_state_blocked_pr"),
        trusted_reimplement_command("IC_reimplement_blocked_pr"),
      })
    end,
    prepare_extra = function()
      entity_read_mocks.mock_pr_view_selector(t, {
        repo = REPO,
        number = 7,
        head = "devloop-owner-repo-42-01HY",
        head_sha = "def456",
        base_branch = "dev",
        state = "OPEN",
        comments = json_array(),
        labels = json_array(),
      }, entity_read_mocks.pr_origin_selector, 1)
    end,
    reason_code = "operator-reimplement-blocked-open-pr",
  },
  {
    name = "blocked-implementing-timeout-without-pr",
    state = "blocked",
    state_version = BLOCKED_TIMEOUT_VERSION,
    labels = { "fkst-dev:enabled", "fkst-dev:blocked" },
    comments = function()
      return json_array({
        trusted_comment(core.state_marker(PROPOSAL_ID, "implementing", READY_VERSION), "IC_state_implementing"),
        trusted_comment(core.state_marker(PROPOSAL_ID, "blocked", BLOCKED_TIMEOUT_VERSION), "IC_state_blocked_timeout"),
        trusted_comment(conv_reconcile.timeout_reconcile_marker(
          PROPOSAL_ID,
          READY_VERSION,
          "implementing",
          3,
          "drop",
          {
            terminal_version = BLOCKED_TIMEOUT_VERSION,
            from_state = "implementing",
            from_version = READY_VERSION,
            reason_class = "state-output-obligation-timeout",
            source_ref = SOURCE_REF,
          }
        ), "IC_timeout_reconcile"),
        trusted_reimplement_command("IC_reimplement_blocked_timeout"),
      })
    end,
    reason_code = "operator-reimplement-blocked-implementing-timeout-without-pr",
    timeout_evidence_source = "trusted-timeout-reconcile-marker",
  },
}

local function event_for(fixture)
  return {
    queue = "github-proxy.github_entity_changed",
    ts = "2026-06-03T02:03:04Z",
    payload = h.issue({
      labels = fixture.labels,
      updated_at = "2026-06-03T02:03:04Z",
      dedup_key = "owner/repo#issue#42@2026-06-03T02:03:04Z/" .. fixture.name,
      source_ref = copy_value(SOURCE_REF),
    }),
  }
end

local function prepare_fixture(fixture)
  h.mock_bot_env()
  entity_read_mocks.mock_issue_read_forms(t, {
    repo = REPO,
    number = ISSUE_NUMBER,
    title = "Capture direct ready payload construction",
    updated_at = "2026-06-03T02:03:04Z",
    state = "OPEN",
    labels = fixture.labels,
    comments = fixture.comments(),
    assignees = { "fkst-test-bot" },
    author_login = "fkst-test-bot",
    times = 1,
  })
  if fixture.prepare_extra ~= nil then
    fixture.prepare_extra()
  end
end

local function capture_runtime(fixture)
  prepare_fixture(fixture)
  local event = event_for(fixture)
  local constructor_calls = json_array()
  local decisions = json_array()
  local original_builder = payloads_builders.build_devloop_ready_payload
  local original_decision = devloop_logging.log_cas_decision
  local original_write_mode = config.write_mode
  local original_read_env = devloop_base.read_env

  payloads_builders.build_devloop_ready_payload = function(M, source)
    local payload = original_builder(M, source)
    table.insert(constructor_calls, {
      source = copy_value(source),
      payload = copy_value(payload),
    })
    return payload
  end
  devloop_logging.log_cas_decision = function(dept, proposal_id, current, from_state, to_state, outcome, reason)
    if dept == "observe_issue" and outcome == "applied(operator-reimplement)" then
      table.insert(decisions, {
        proposal_id = proposal_id,
        current = copy_value(current),
        from_state = from_state,
        to_state = to_state,
        outcome = outcome,
        reason = reason,
      })
    end
    return original_decision(dept, proposal_id, current, from_state, to_state, outcome, reason)
  end
  config.write_mode = function()
    return "real"
  end
  devloop_base.read_env = function(name, exec)
    local values = {
      FKST_GITHUB_BOT_LOGIN = "fkst-test-bot",
      FKST_GITHUB_WRITE = "1",
      FKST_DEVLOOP_MANAGED_BOT_LOGINS = "fkst-test-bot",
      FKST_GITHUB_AUTHORIZED_LOGINS = "trusted-human",
    }
    if values[name] ~= nil then
      return values[name]
    end
    return original_read_env(name, exec)
  end

  local ok, result = pcall(testing.run_fake, observe_issue_department, event)
  devloop_base.read_env = original_read_env
  config.write_mode = original_write_mode
  devloop_logging.log_cas_decision = original_decision
  payloads_builders.build_devloop_ready_payload = original_builder
  if not ok then
    error(result, 0)
  end

  t.eq(#constructor_calls, 1, fixture.name .. ": real dispatch calls the constructor exactly once")
  t.eq(#decisions, 1, fixture.name .. ": real dispatch reaches the operator reimplement branch")
  local ready_raises = json_array()
  for _, raised in ipairs(result.raises) do
    if raised.queue == "devloop_ready" then
      table.insert(ready_raises, copy_value(raised))
    end
  end
  t.eq(#ready_raises, 1, fixture.name .. ": real dispatch emits exactly one devloop_ready payload")
  t.eq(
    canonical_json(ready_raises[1].payload),
    canonical_json(constructor_calls[1].payload),
    fixture.name .. ": emitted payload exactly matches the direct constructor result"
  )
  return event, constructor_calls[1], decisions[1], ready_raises[1]
end

local function build_record(fixture)
  local event, constructor, decision, ready_raise = capture_runtime(fixture)
  local source = constructor.source
  local payload = constructor.payload
  local operator_reentry = payload.operator_reentry
  local timeout_source = fixture.timeout_evidence_source and fixture.timeout_evidence_source or JSON_NULL
  return {
    schema = "restart-old-behavior-observation.v2",
    observation_id = "ctor:github-devloop:observe-issue-ready/" .. fixture.name,
    owner = "github-devloop",
    site = copy_value(SITE),
    boundary = "writer",
    typed_intent = {
      kind = "direct_constructor",
      source_state = decision.current.state,
      source_boundary = event.queue,
      target = "devloop_ready",
      cause_schema_id = event.payload.schema,
      generation_epoch = {
        current_version = decision.current.version,
        source_version = source.dedup_key,
        payload_version = payload.dedup_key,
        impl_retry_attempt = payload.impl_retry_attempt,
      },
      lineage = {
        proposal_id = payload.proposal_id,
        issue_number = event.payload.number,
        source_ref = copy_value(payload.source_ref),
        operator_reentry = nullable(operator_reentry),
      },
    },
    old_inputs = {
      current_fact = {
        state = decision.current.state,
        version = decision.current.version,
        stage_rank = decision.current.stage_rank,
      },
      caller_from_states = json_array({ fixture.state }),
      incoming_version = source.dedup_key,
      target_version = payload.dedup_key,
      handoff_reference = nullable(operator_reentry),
    },
    old_outcome = {
      status = "constructed",
      reason_code = fixture.reason_code,
      cas_outcome = "not-applicable-direct-constructor",
      emitted_effects = json_array({
        {
          effect_id = "queue:devloop_ready",
          sink_kind = "queue",
          authority_class = "lifecycle-authoritative",
          ordinal = 1,
        },
      }),
      observable_writes = json_array({
        {
          effect_id = "queue:devloop_ready",
          queue = ready_raise.queue,
          payload = copy_value(ready_raise.payload),
        },
      }),
      handoff_direct_lookup_count = 0,
      timeout_evidence_source = timeout_source,
    },
    evidence_refs = json_array({
      {
        kind = "runtime-constructor-call",
        ref = "devloop.payloads.builders.build_devloop_ready_payload",
      },
      {
        kind = "runtime-event-source",
        ref = event.payload.source_ref.ref,
      },
      {
        kind = "production-callsite",
        ref = "packages/github-devloop/departments/observe_issue/main.lua:551",
      },
    }),
  }
end

local function capture_records()
  local records = json_array()
  for _, fixture in ipairs(FIXTURES) do
    table.insert(records, build_record(fixture))
  end
  table.sort(records, function(left, right)
    return left.observation_id < right.observation_id
  end)
  return records
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
  test_build_devloop_ready_payload_old_observations_are_real_dispatch_runtime_bound_and_bidirectional = function()
    local first = capture_records()
    local second = capture_records()
    local repeat_difference = first_difference(
      second,
      first,
      "old_behavior_observations[observe-issue-ready-constructor][repeat]"
    )
    if repeat_difference ~= nil or canonical_json(second) ~= canonical_json(first) then
      error("second OLD direct-constructor runtime capture differs at " .. tostring(repeat_difference or "canonical-json"), 0)
    end

    t.eq(#first, #FIXTURES, "every production branch at the constructor site produces one payload variant")
    local expected = committed_records()
    local inventory_difference = first_difference(
      first,
      expected,
      "old_behavior_observations[observe-issue-ready-constructor]"
    )
    if inventory_difference ~= nil or canonical_json(first) ~= canonical_json(expected) then
      error(
        "runtime-bound OLD direct-constructor observation differs at "
          .. tostring(inventory_difference or "canonical-json")
          .. "; runtime_records=" .. canonical_json(first),
        0
      )
    end
  end,
}
