local config = require("devloop.config")
local conv_attempts = require("devloop.convergence.attempts")
local conv_reconcile = require("devloop.convergence.reconcile")
local devloop_logging = require("devloop.logging")
local devloop_state = require("devloop.state")
local entity_lib = require("devloop.entity")
local entity_read_mocks = require("tests.entity_read_mock_helpers")
local h = require("tests.devloop_helpers")
local m_rae = require("devloop.restart_actionable_epoch")
local m_mgw = require("devloop.merge_gate_wait")
local observation_support = require("testkit.old_behavior_observation_support")
local replay_fields = require("devloop.replay_fields")
local testing = require("testkit.testing")
local reconcile_department = require("departments.reconcile.main")

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
  path = "packages/github-devloop-pr/departments/reconcile/main.lua",
  symbol = "pipeline_timeout",
  ordinal = "versioned_transition_status:reconcile.state->blocked",
}

local REPO = "owner/repo"
local ISSUE_NUMBER = 42
local PR_NUMBER = 7
local HEAD_SHA = "def456"
local NOW_SECONDS = 1784048400
local OLD_CREATED_AT = "2026-06-03T01:00:00Z"
local RECENT_CREATED_AT = "2026-07-14T16:59:00Z"
local SOURCE_STATES = { "pr-open", "reviewing", "fixing", "review-meta", "merge-ready", "merging" }

local function restart_row(state_name)
  return replay_fields.restart_transition_row(core.restart_transition_table(), state_name)
end

local function trusted_comment(body, created_at)
  return {
    body = body,
    author_login = "fkst-test-bot",
    created_at = created_at or OLD_CREATED_AT,
  }
end

local function state_comment(proposal_id, state_name, version, created_at)
  return trusted_comment(core.state_marker(proposal_id, state_name, version), created_at)
end

local function timeout_event(state_name, options)
  options = options or {}
  local seed = h.fix_reconcile()
  local source_ref = options.source_ref or entity_lib.pr_source_ref(REPO, PR_NUMBER)
  local state_version = options.state_version
    or seed.issue_version .. "/timeout/" .. state_name .. "/3"
  local round = options.round or 3
  local payload = conv_reconcile.build_devloop_timeout_reconcile_payload(
    restart_row(state_name),
    { state = state_name, version = state_version },
    seed.proposal_id,
    source_ref,
    round
  )
  return {
    queue = "devloop_timeout_reconcile",
    payload = payload,
    now_seconds = NOW_SECONDS,
  }
end

local function with_no_codex_runs(fn)
  local original = fkst.codex_runs
  fkst.codex_runs = function()
    return { running = json_array(), recent = json_array() }
  end
  local ok, result = pcall(fn)
  fkst.codex_runs = original
  if not ok then error(result, 0) end
  return result
end

local function fixing_attempt_comments(event, comments)
  local row = restart_row("fixing")
  local state = {
    state = "fixing",
    version = event.payload.issue_version,
    marker_created_at = OLD_CREATED_AT,
    proposal_id = event.payload.proposal_id,
  }
  local facts = {
    proposal_id = event.payload.proposal_id,
    current = { comments = comments },
    current_pr = { head_sha = HEAD_SHA, comments = comments },
    source_ref = event.payload.source_ref,
    head_sha = HEAD_SHA,
    fresh_current_state = state,
  }
  local eval = with_no_codex_runs(function()
    return m_rae.actionable_epoch_resolve(core, row, state, facts, NOW_SECONDS)
  end)
  t.eq(eval.status, "actionable", "fixing fixture has an actionable runtime epoch")
  t.is_true(eval.generation_key ~= nil, "fixing fixture has a runtime generation key")
  for round = 1, 2 do
    table.insert(comments, trusted_comment(conv_attempts.timeout_attempt_v2_marker(
      event.payload.proposal_id,
      row.from_state,
      row.liveness_class_id,
      eval.generation_key,
      round,
      event.payload.source_ref
    )))
  end
end

local function apply_comments(event, state_name)
  local comments = json_array({
    state_comment(event.payload.proposal_id, state_name, event.payload.issue_version),
  })
  if state_name == "fixing" then
    fixing_attempt_comments(event, comments)
  end
  return comments
end

local function prepare_fixture(comments, surface)
  h.mock_bot_env()
  h.mock_default_issue_claim(REPO, ISSUE_NUMBER)
  if surface == "issue-fallback" then
    entity_read_mocks.mock_pr_view_raw_selector(t, {
      repo = REPO,
      number = PR_NUMBER,
    }, entity_read_mocks.pr_origin_selector, {
      stdout = "",
      stderr = "HTTP 404: Not Found",
      exit_code = 1,
    }, 1)
    entity_read_mocks.mock_issue_read_forms(t, {
      repo = REPO,
      number = ISSUE_NUMBER,
      comments = comments,
      labels = {},
      assignees = { "fkst-test-bot" },
      author_login = "fkst-test-bot",
      register_all_views = true,
      times = 1,
    })
    return
  end
  if surface == "issue" then
    entity_read_mocks.mock_issue_read_forms(t, {
      repo = REPO,
      number = ISSUE_NUMBER,
      comments = comments,
      labels = {},
      assignees = { "fkst-test-bot" },
      author_login = "fkst-test-bot",
      register_all_views = true,
      times = 1,
    })
    return
  end
  entity_read_mocks.mock_pr_view_selector(t, {
    repo = REPO,
    number = PR_NUMBER,
    comments = comments,
    head = "devloop-owner-repo-42-01HY",
    head_sha = HEAD_SHA,
    state = "OPEN",
    base_branch = "dev",
    labels = {},
  }, entity_read_mocks.pr_origin_selector, 1)
end

local function observe_real_department(event, comments, surface)
  prepare_fixture(comments, surface)
  return observation_support.observe_department({
    config = config,
    devloop_logging = devloop_logging,
    devloop_state = devloop_state,
    dept = "reconcile",
    from_state = event.payload.state,
    transition_kind = "versioned_transition_status",
    run = function()
      local original_now = now
      now = function() return NOW_SECONDS end
      local ok, result = pcall(testing.run_fake, reconcile_department, event)
      now = original_now
      if not ok then error(result, 0) end
      return result
    end,
    codex_runs_for_read = json_array(),
    write_mode = "real",
  })
end

local EFFECTS = {
  ["github-proxy.github_issue_comment_request"] = {
    effect_id = "comment:issue:timeout-reconcile",
    sink_kind = "comment",
    authority_class = "lifecycle-authoritative",
  },
  ["github-proxy.github_pr_comment_request"] = {
    effect_id = "comment:pr:timeout-reconcile",
    sink_kind = "comment",
    authority_class = "lifecycle-authoritative",
  },
  ["github-proxy.github_issue_label_request"] = {
    effect_id = "label:issue:timeout-reconcile",
    sink_kind = "label",
    authority_class = "lifecycle-authoritative",
  },
}

local function effects_from_raises(raises)
  local effects = json_array()
  local writes = json_array()
  for ordinal, raised in ipairs(raises) do
    local shape = EFFECTS[raised.queue]
    if shape == nil then
      error("unclassified OLD timeout reconcile raise: " .. tostring(raised.queue))
    end
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

local function record_lineage(payload, incoming_version, decision)
  return {
    proposal_id = decision.proposal_id,
    state = payload.state,
    issue_version = payload.issue_version,
    round = payload.round,
    dedup_key = payload.dedup_key,
    incoming_version = incoming_version,
    source_ref = copy_value(payload.source_ref),
  }
end

local function apply_outcome_status(probe, decision, apply)
  t.eq(probe.outcome, "apply", "timeout reconcile reaches the apply CAS regime")
  t.eq(decision.outcome, "applied", "timeout reconcile routes the apply decision")
  t.eq(apply.to_state, "blocked", "timeout reconcile applies blocked")
  return "apply", "apply", decision.outcome
end

local function build_apply_record(source_state, surface, event, result, captured, options)
  options = options or {}
  local writer_capture = captured
  if surface == "issue-fallback" then
    t.eq(#captured.decisions, 2, source_state .. ": fallback and apply decisions are both observed")
    t.eq(captured.decisions[1].outcome, "pr-surface-gone-fallback", source_state .. ": PR 404 fallback disposition")
    if captured.decisions[2].outcome ~= "applied" then
      error(source_state .. ": fallback did not reach writer apply: " .. canonical_json(captured.decisions[2]), 0)
    end
    writer_capture = copy_value(captured)
    writer_capture.decisions = json_array({ captured.decisions[2] })
  end
  local record = observation_support.build_record({
    t = t,
    dept = "reconcile",
    event = event,
    result = result,
    captured = writer_capture,
    owner = "github-devloop-pr",
    site = SITE,
    observation_prefix = "writer:github-devloop-pr:reconcile-state-blocked",
    observation_variant = options.observation_variant or source_state .. "-" .. surface .. "-apply",
    transition_kind = "versioned_transition_status",
    source_state = function(probe) return probe.current.state end,
    outcome_status = function(probe, decision, apply)
      local status, reason_code, cas_outcome = apply_outcome_status(probe, decision, apply)
      if surface == "issue-fallback" then
        reason_code = "pr-surface-gone-fallback"
      end
      if options.reason_code ~= nil then
        reason_code = options.reason_code
      end
      return status, reason_code, cas_outcome
    end,
    effects_from_raises = effects_from_raises,
    lineage = function(payload, probe, decision)
      return record_lineage(payload, probe.incoming_version, decision)
    end,
  })
  if surface == "issue-fallback" then
    table.insert(record.evidence_refs, {
      kind = "runtime-cas-decision",
      ref = "devloop.logging.log_cas_decision:pr-surface-gone-fallback",
    })
  end
  return record
end

local function capture_apply_record(source_state, surface, options)
  options = options or {}
  local event = timeout_event(source_state)
  local comments = apply_comments(event, source_state)
  if options.external_ci_wait == true then
    table.insert(comments, trusted_comment(m_mgw.merge_gate_wait_marker(
      event.payload.proposal_id,
      PR_NUMBER,
      m_mgw.merge_gate_wait_version_lineage(event.payload.issue_version),
      HEAD_SHA,
      "ci-wait",
      "CI_WAIT"
    )))
  end
  local result, captured = observe_real_department(event, comments, surface)
  local record = build_apply_record(source_state, surface, event, result, captured, options)
  local expected_version = conv_reconcile.timeout_reconcile_state_version(
    event.payload.issue_version,
    source_state,
    event.payload.round
  )

  local name = source_state .. "/" .. surface
  t.eq(#captured.probes, 1, name .. ": one writer CAS probe")
  t.eq(captured.probes[1].incoming_version, expected_version, name .. ": helper-derived incoming version")
  t.is_true(expected_version ~= event.payload.issue_version, name .. ": helper appends a terminal version segment")
  t.eq(canonical_json(captured.probes[1].from_states), canonical_json(json_array({ source_state })), name .. ": singleton source CAS")
  t.eq(record.typed_intent.source_state, source_state, name .. ": captured runtime source")
  t.eq(record.old_inputs.target_version, JSON_NULL, name .. ": four-argument target_version")
  t.eq(record.old_outcome.status, "apply", name .. ": captured status")
  t.eq(record.old_outcome.cas_outcome, "applied", name .. ": captured CAS outcome")
  t.eq(
    record.old_outcome.reason_code,
    options.reason_code or (surface == "issue-fallback" and "pr-surface-gone-fallback" or "apply"),
    name .. ": exact path disposition"
  )
  t.eq(#record.old_outcome.emitted_effects, 2, name .. ": comment and label effects")
  if options.external_ci_wait == true then
    t.is_true(
      result.raises[1].payload.body:find("reason_class=external-ci-wait-expired", 1, true) ~= nil,
      name .. ": external CI wait reason reaches the timeout comment"
    )
  end
  return record
end

local function build_guard_record(fixture, event, result, captured, observation_variant)
  local decision = captured.decisions[1]
  local current = decision.current
  local incoming_version = conv_reconcile.timeout_reconcile_state_version(
    event.payload.issue_version,
    event.payload.state,
    event.payload.round
  )
  local emitted_effects, observable_writes = effects_from_raises(result.raises)
  return {
    schema = "restart-old-behavior-observation.v2",
    observation_id = table.concat({
      "writer:github-devloop-pr:reconcile-state-blocked",
      observation_variant or fixture.name,
      "blocked",
      fixture.status,
      fixture.reason_code,
      "none",
    }, "/"),
    owner = "github-devloop-pr",
    site = copy_value(SITE),
    boundary = "writer",
    typed_intent = {
      kind = "versioned_transition_status",
      source_state = nullable(current.state),
      source_boundary = JSON_NULL,
      target = "blocked",
      cause_schema_id = event.payload.schema,
      generation_epoch = {
        current_version = nullable(current.version),
        incoming_version = incoming_version,
        target_version = JSON_NULL,
      },
      lineage = record_lineage(event.payload, incoming_version, decision),
    },
    old_inputs = {
      current_fact = {
        state = nullable(current.state),
        version = nullable(current.version),
        stage_rank = nullable(current.stage_rank),
      },
      caller_from_states = json_array({ event.payload.state }),
      incoming_version = incoming_version,
      target_version = JSON_NULL,
      handoff_reference = JSON_NULL,
    },
    old_outcome = {
      status = fixture.status,
      reason_code = fixture.reason_code,
      cas_outcome = decision.outcome,
      emitted_effects = emitted_effects,
      observable_writes = observable_writes,
      handoff_direct_lookup_count = captured.handoff_direct_lookup_count,
      timeout_evidence_source = JSON_NULL,
    },
    evidence_refs = json_array({
      {
        kind = "runtime-cas-decision",
        ref = "devloop.logging.log_cas_decision:" .. tostring(decision.outcome),
      },
      {
        kind = "runtime-event-source",
        ref = tostring(event.payload.source_ref and event.payload.source_ref.ref),
      },
    }),
  }
end

local GUARDS = {
  {
    name = "timeout-reconcile-marker-visible",
    status = "skip-idempotent(timeout reconcile marker already visible)",
    reason_code = "timeout-reconcile-marker-visible",
    comments = function(event)
      local terminal_version = conv_reconcile.timeout_reconcile_state_version(
        event.payload.issue_version,
        event.payload.state,
        event.payload.round
      )
      return json_array({
        state_comment(event.payload.proposal_id, event.payload.state, event.payload.issue_version),
        trusted_comment(conv_reconcile.timeout_reconcile_marker(
          event.payload.proposal_id,
          event.payload.issue_version,
          event.payload.state,
          event.payload.round,
          "drop",
          { terminal_version = terminal_version }
        )),
      })
    end,
  },
  {
    name = "already-terminal",
    status = "skip-idempotent(already terminal)",
    reason_code = "already-terminal",
    comments = function(event)
      return json_array({
        state_comment(event.payload.proposal_id, "merged", event.payload.issue_version .. "/merged"),
      })
    end,
  },
  {
    name = "state-advanced",
    status = "skip-stale(state-advanced)",
    reason_code = "state-advanced",
    comments = function(event, source_state)
      local mismatched_state = ({
        ["pr-open"] = "reviewing",
        reviewing = "fixing",
        fixing = "review-meta",
        ["review-meta"] = "fixing",
        ["merge-ready"] = "merging",
        merging = "fixing",
      })[source_state]
      return json_array({
        state_comment(event.payload.proposal_id, mismatched_state, event.payload.issue_version),
      })
    end,
  },
  {
    name = "lineage-mismatch",
    status = "skip-stale(lineage-mismatch)",
    reason_code = "lineage-mismatch",
    comments = function(event)
      return json_array({
        state_comment(
          event.payload.proposal_id,
          event.payload.state,
          "ready/consensus-github-devloop/issue/owner/repo/42/2026-06-04T01-02-03Z/timeout/"
            .. event.payload.state .. "/3"
        ),
      })
    end,
  },
  {
    name = "no-longer-over-budget",
    status = "skip-stale(no-longer-over-budget)",
    reason_code = "no-longer-over-budget",
    event = function(source_state)
      return timeout_event(source_state, {
        state_version = "ready/consensus-github-devloop/issue/owner/repo/42/2026-07-14T16-59-00Z/timeout/"
          .. source_state .. "/3",
      })
    end,
    comments = function(event)
      return json_array({
        state_comment(event.payload.proposal_id, event.payload.state, event.payload.issue_version, RECENT_CREATED_AT),
      })
    end,
  },
}

local function capture_guard(source_state, fixture)
  local event = fixture.event and fixture.event(source_state) or timeout_event(source_state)
  local name = source_state .. "/" .. fixture.name
  local result, captured = observe_real_department(event, fixture.comments(event, source_state))
  t.eq(#captured.probes, 0, name .. ": returns before writer CAS")
  t.eq(#captured.decisions, 1, name .. ": one returning disposition")
  t.eq(captured.decisions[1].from_state, event.payload.state, name .. ": source state")
  t.eq(captured.decisions[1].to_state, "blocked", name .. ": target state")
  t.eq(captured.decisions[1].outcome, fixture.status, name .. ": exact disposition")
  t.eq(#captured.applies, 0, name .. ": no apply")
  t.eq(#captured.raises, 0, name .. ": no logged raise")
  t.eq(#result.raises, 0, name .. ": no runtime effect")
  local record = build_guard_record(fixture, event, result, captured, source_state .. "-" .. fixture.name)
  t.eq(record.old_inputs.target_version, JSON_NULL, name .. ": four-argument target_version")
  t.eq(#record.old_outcome.emitted_effects, 0, name .. ": no emitted effects")
  t.eq(#record.old_outcome.observable_writes, 0, name .. ": no observable writes")
  return record
end

local function capture_blocked_family(surface, marker_visible)
  local source_ref = surface == "issue"
    and entity_lib.issue_source_ref(REPO, ISSUE_NUMBER)
    or entity_lib.pr_source_ref(REPO, PR_NUMBER)
  local event = timeout_event("blocked", { source_ref = source_ref })
  local comments = apply_comments(event, "blocked")
  if marker_visible then
    table.insert(comments, trusted_comment(conv_attempts.decompose_exhausted_marker(
      event.payload.proposal_id,
      event.payload.issue_version,
      event.payload.round,
      event.payload.source_ref
    )))
  end
  local result, captured = observe_real_department(event, comments, surface)
  local name = "blocked/" .. surface .. (marker_visible and "/marker-visible" or "/apply")
  t.eq(#captured.probes, 0, name .. ": blocked subfamily bypasses state-to-blocked CAS")
  t.eq(#captured.decisions, 1, name .. ": one blocked subfamily disposition")
  t.eq(captured.decisions[1].from_state, "blocked", name .. ": blocked source")
  t.eq(captured.decisions[1].to_state, "devloop_decompose", name .. ": decompose target")
  if marker_visible then
    t.eq(captured.decisions[1].outcome, "skip-idempotent(decompose-exhausted)", name .. ": exact guard")
    t.eq(#result.raises, 0, name .. ": marker-visible emits no effect")
  else
    t.eq(captured.decisions[1].outcome, "applied(decompose-exhausted)", name .. ": exact apply")
    t.eq(#result.raises, 1, name .. ": decompose exhaustion emits one comment")
    t.eq(
      result.raises[1].queue,
      surface == "issue" and "github-proxy.github_issue_comment_request"
        or "github-proxy.github_pr_comment_request",
      name .. ": exact surface sink"
    )
  end
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
    if is_target_record(record) then table.insert(selected, record) end
  end
  table.sort(selected, function(left, right)
    return tostring(left.observation_id) < tostring(right.observation_id)
  end)
  return selected
end

return {
  test_timeout_reconcile_old_observations_are_runtime_bound_to_inventory = function()
    local actual = json_array()
    for _, source_state in ipairs(SOURCE_STATES) do
      for _, fixture in ipairs(GUARDS) do
        table.insert(actual, capture_guard(source_state, fixture))
      end
      table.insert(actual, capture_apply_record(source_state, "pr"))
      table.insert(actual, capture_apply_record(source_state, "issue-fallback"))
    end
    for _, source_state in ipairs({ "merge-ready", "merging" }) do
      table.insert(actual, capture_apply_record(source_state, "pr", {
        external_ci_wait = true,
        observation_variant = source_state .. "-pr-external-ci-wait-apply",
        reason_code = "external-ci-wait-expired",
      }))
    end
    table.sort(actual, function(left, right)
      return tostring(left.observation_id) < tostring(right.observation_id)
    end)
    local expected = committed_records()
    local difference = first_difference(actual, expected, "old_behavior_observations[reconcile_timeout]")
    if difference ~= nil or canonical_json(actual) ~= canonical_json(expected) then
      error(
        "runtime-bound OLD timeout reconcile observation differs at " .. tostring(difference or "canonical-json")
          .. "; runtime_records=" .. canonical_json(actual),
        0
      )
    end
  end,
  test_timeout_reconcile_blocked_payload_family_is_runtime_classified = function()
    capture_blocked_family("pr", false)
    capture_blocked_family("issue", false)
    capture_blocked_family("pr", true)
  end,
}
