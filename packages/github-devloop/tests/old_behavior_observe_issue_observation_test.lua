local base_ids = require("devloop.base_ids")
local config = require("devloop.config")
local devloop_base = require("devloop.base")
local devloop_forks = require("devloop.forks")
local devloop_logging = require("devloop.logging")
local devloop_state = require("devloop.state")
local entity_read_mocks = require("tests.entity_read_mock_helpers")
local h = require("tests.devloop_helpers")
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
  ordinal = "versioned_transition_status:unmanaged->thinking",
}

local REPO = "owner/repo"
local ISSUE_NUMBER = 42
local PROPOSAL_ID = "github-devloop/issue/owner/repo/42"
local T_OLDER = "2026-06-02T01:02:03Z"
local T_EQUAL = "2026-06-03T01:02:03Z"
local T_NOT_TIMEOUT_DUE = "2099-01-01T00:00:00Z"

local function fork_parent_comment(repo, issue_number)
  return {
    body = '<!-- fkst:github-proxy:issue-created:v1 dedup="'
      .. devloop_forks.fork_issue_dedup_key(repo or REPO, issue_number or ISSUE_NUMBER)
      .. '" issue="99" -->',
    author_login = "fkst-test-bot",
  }
end

local function issue_payload(updated_at, issue_number)
  local timestamp = updated_at or T_EQUAL
  local number = issue_number or ISSUE_NUMBER
  return h.issue({
    number = number,
    updated_at = timestamp,
    dedup_key = "owner/repo#issue#" .. tostring(number) .. "@" .. timestamp,
    source_ref = {
      kind = "external",
      ref = "owner/repo#issue/" .. tostring(number),
    },
  })
end

local function issue_event(updated_at, issue_number)
  return {
    queue = "github-proxy.github_entity_changed",
    payload = issue_payload(updated_at, issue_number),
    now_seconds = 1780448523,
  }
end

local function proposal_version(updated_at)
  return payloads_builders.build_proposal(issue_payload(updated_at)).dedup_key
end

local function state_comment(state, version)
  return {
    body = core.state_marker(PROPOSAL_ID, state, version),
    author_login = "fkst-test-bot",
    created_at = T_NOT_TIMEOUT_DUE,
  }
end

local function prepare_fixture(event, fixture)
  local repo = event.payload.repo
  local issue_number = event.payload.number
  h.mock_bot_env()
  if fixture.unsupported_payload then
    return
  end
  entity_read_mocks.mock_issue_read_forms(t, {
    repo = repo,
    number = issue_number,
    title = event.payload.title,
    updated_at = event.payload.updated_at,
    state = fixture.issue_state or "OPEN",
    labels = fixture.labels,
    comments = fixture.comments and fixture.comments(event) or json_array(),
    assignees = fixture.assignees or { "fkst-test-bot" },
    author_login = fixture.author_login or "fkst-test-bot",
    created_at = fixture.created_at or "2026-06-01T00:00:00Z",
    times = 1,
  })
  if fixture.rederived then
    local rederived = fixture.rederived(event)
    t.mock_command(core.gh_issue_view_state_cmd(repo, issue_number), {
      stdout = entity_read_mocks.issue_view_stdout({
        repo = repo,
        number = issue_number,
        title = event.payload.title,
        state = rederived.state or "OPEN",
        labels = rederived.labels or fixture.labels,
        comments = rederived.comments or json_array(),
        assignees = rederived.assignees or json_array(),
        author_login = rederived.author_login,
        created_at = rederived.created_at or fixture.created_at or "2026-06-01T00:00:00Z",
        updated_at = event.payload.updated_at,
      }),
      stderr = "",
      exit_code = 0,
    })
  end
  if fixture.claim_commands then
    fixture.claim_commands(event)
  end
  if fixture.needs_context then
    h.mock_context_bundle(event.payload)
  end
end

local function controlled_codex_runs(event, fixture)
  if not fixture.live_thinking then
    return json_array()
  end
  return json_array({
    {
      role = "consensus",
      proposal_id = PROPOSAL_ID,
      dedup_key = proposal_version(T_EQUAL),
      status = "running",
    },
  })
end

local function is_unmanaged_thinking_probe(probe)
  if type(probe) ~= "table" or probe.to_state ~= "thinking" then
    return false
  end
  for _, from_state in ipairs(probe.from_states or {}) do
    if from_state == "unmanaged" then
      return true
    end
  end
  return false
end

local function scope_unmanaged_thinking_probes(captured)
  local selected = json_array()
  local excluded = json_array()
  for _, probe in ipairs(captured.probes or {}) do
    if is_unmanaged_thinking_probe(probe) then
      table.insert(selected, probe)
    else
      table.insert(excluded, probe)
    end
  end
  captured.site_probes = selected
  captured.excluded_probes = excluded
end

local INTAKE_DECISION_REASONS = {
  ["unsupported event payload"] = true,
  ["issue is not open"] = true,
  ["fkst-dev:enabled label is absent"] = true,
  ["fkst-dev:hold label is present"] = true,
  ["current marker is not an unmanaged start"] = true,
  ["unmanaged state marker pending for observe"] = true,
  ["starting consensus for opted-in issue"] = true,
}

local function scope_unmanaged_thinking_decisions(captured, fixture)
  local selected = json_array()
  local excluded = json_array()
  for _, decision in ipairs(captured.decisions or {}) do
    local is_selected = (fixture.decision_from_state ~= nil
      and decision.from_state == fixture.decision_from_state)
      or (decision.from_state == "unmanaged"
        and decision.to_state == "thinking"
        and INTAKE_DECISION_REASONS[decision.reason] == true)
    if is_selected then
      table.insert(selected, decision)
    else
      table.insert(excluded, decision)
    end
  end
  captured.site_decisions = selected
  captured.excluded_decisions = excluded
end

local function site_capture(captured)
  local scoped = {}
  for key, value in pairs(captured) do
    scoped[key] = value
  end
  scoped.probes = captured.site_probes
  scoped.decisions = captured.site_decisions
  return scoped
end

local function observe_real_department(event, fixture)
  prepare_fixture(event, fixture)
  local original_claim_mode = config.claim_mode
  local original_read_env = devloop_base.read_env
  local run_error = nil
  config.claim_mode = function()
    return fixture.claim_mode or "assignee"
  end
  devloop_base.read_env = function(name, exec)
    local values = {
      FKST_GITHUB_BOT_LOGIN = "fkst-test-bot",
      FKST_GITHUB_WRITE = fixture.write_mode or "",
      FKST_DEVLOOP_FORK_GRACE_HOURS = "",
      FKST_DEVLOOP_MANAGED_BOT_LOGINS = fixture.managed_bot_logins or "fkst-test-bot",
      FKST_GITHUB_AUTHORIZED_LOGINS = fixture.authorized_logins or "trusted-human",
    }
    if values[name] ~= nil then
      return values[name]
    end
    return original_read_env(name, exec)
  end
  local ok, result, captured = pcall(observation_support.observe_department, {
    config = config,
    devloop_logging = devloop_logging,
    devloop_state = devloop_state,
    dept = "observe_issue",
    from_state = fixture.decision_from_state or "unmanaged",
    transition_kind = "versioned_transition_status",
    run = function()
      local run_ok, run_result = pcall(testing.run_fake, observe_issue_department, event)
      if fixture.error_contains ~= nil then
        if run_ok then
          error(fixture.name .. ": expected department error containing " .. fixture.error_contains)
        end
        run_error = tostring(run_result)
        return { raises = json_array() }
      end
      if not run_ok then
        error(run_result, 0)
      end
      return run_result
    end,
    codex_runs_for_read = controlled_codex_runs(event, fixture),
    write_mode = "real",
  })
  devloop_base.read_env = original_read_env
  config.claim_mode = original_claim_mode
  if not ok then
    error(result, 0)
  end
  captured.run_error = run_error
  scope_unmanaged_thinking_probes(captured)
  scope_unmanaged_thinking_decisions(captured, fixture)
  local expected_excluded = fixture.excluded_probe_count or 0
  if #captured.excluded_probes ~= expected_excluded then
    error(fixture.name .. ": unexpected other-transition probe count; expected="
      .. tostring(expected_excluded) .. "; selected=" .. canonical_json(captured.site_probes)
      .. "; excluded=" .. canonical_json(captured.excluded_probes))
  end
  local expected_excluded_decisions = fixture.excluded_decision_count or 0
  if #captured.excluded_decisions ~= expected_excluded_decisions then
    error(fixture.name .. ": unexpected other-transition decision count; expected="
      .. tostring(expected_excluded_decisions) .. "; selected=" .. canonical_json(captured.site_decisions)
      .. "; excluded=" .. canonical_json(captured.excluded_decisions))
  end
  return result, captured
end

local EFFECTS = {
  ["consensus.proposal"] = {
    effect_id = "queue:consensus.proposal",
    sink_kind = "queue",
    authority_class = "lifecycle-authoritative",
  },
  ["github-proxy.github_issue_comment_request"] = {
    effect_id = "comment:issue:thinking-state",
    sink_kind = "comment",
    authority_class = "lifecycle-authoritative",
  },
  ["github-proxy.github_issue_label_request"] = {
    effect_id = "label:issue:thinking-state",
    sink_kind = "label",
    authority_class = "lifecycle-authoritative",
  },
  ["github-proxy.github_issue_create_request"] = {
    effect_id = "adapter:github.issue-create-fork",
    sink_kind = "adapter",
    authority_class = "grantless-non-lifecycle",
  },
}

local function effects_from_raises(raises)
  local effects = json_array()
  local writes = json_array()
  for ordinal, raised in ipairs(raises) do
    local shape = EFFECTS[raised.queue]
    if shape == nil then
      error("unclassified OLD observe_issue raise: " .. tostring(raised.queue))
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
    proposal_id = decision and decision.proposal_id or base_ids.proposal_id(payload.repo, payload.number),
    issue_number = payload.number,
    dedup_key = payload.dedup_key,
    incoming_version = incoming_version,
    source_ref = copy_value(payload.source_ref),
  }
end

local function build_cas_record(event, result, captured, fixture)
  if #captured.probes ~= 1 then
    error(fixture.name .. ": expected one unmanaged->thinking versioned probe; site_probes="
      .. canonical_json(captured.probes) .. "; excluded_probes=" .. canonical_json(captured.excluded_probes)
      .. "; decisions=" .. canonical_json(captured.decisions) .. "; lines=" .. canonical_json(captured.lines))
  end
  local record = observation_support.build_record({
    t = t,
    dept = "observe_issue",
    event = event,
    result = result,
    captured = captured,
    owner = "github-devloop",
    site = SITE,
    observation_prefix = "writer:github-devloop:observe-issue-unmanaged-thinking",
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
    outcome_status = function(probe, decision, apply)
      t.eq(probe.outcome, fixture.probe_outcome, fixture.name .. ": real versioned CAS outcome")
      if fixture.error_contains ~= nil then
        t.eq(decision, nil, fixture.name .. ": propagated error logs no claim disposition")
        t.eq(apply, nil, fixture.name .. ": propagated error logs no apply")
        t.is_true(tostring(captured.run_error):find(fixture.error_contains, 1, true) ~= nil,
          fixture.name .. ": exact propagated error class")
        return fixture.status, fixture.reason_code, fixture.cas_outcome
      end
      t.eq(decision.outcome, fixture.decision_outcome, fixture.name .. ": exact writer disposition")
      if fixture.applies_thinking then
        t.eq(apply.to_state, "thinking", fixture.name .. ": effectful route applies thinking")
      else
        t.eq(apply, nil, fixture.name .. ": returning route logs no apply")
      end
      return fixture.status, fixture.reason_code, decision.outcome
    end,
    effects_from_raises = effects_from_raises,
    lineage = function(payload, probe, decision)
      return record_lineage(payload, probe.incoming_version, decision)
    end,
  })

  t.eq(#record.old_outcome.emitted_effects, fixture.effect_count, fixture.name .. ": runtime effect count")
  t.eq(record.old_inputs.target_version, JSON_NULL, fixture.name .. ": four-argument target_version")
  t.eq(record.typed_intent.source_state, fixture.expected_source_state, fixture.name .. ": actual source state")
  t.eq(record.typed_intent.source_boundary, fixture.expected_source_boundary, fixture.name .. ": source boundary")
  return record
end

local function build_guard_record(event, result, captured, fixture)
  t.eq(#captured.probes, 0, fixture.name .. ": guard returns before versioned CAS")
  t.eq(#captured.decisions, 1, fixture.name .. ": guard logs one writer decision")
  t.eq(#captured.applies, 0, fixture.name .. ": guard logs no apply")
  t.eq(#captured.raises, 0, fixture.name .. ": guard logs no raise")
  t.eq(#result.raises, 0, fixture.name .. ": guard emits no effect")
  t.eq(captured.liveness_read_count, 0, fixture.name .. ": guard never reads Codex liveness")

  local decision = captured.decisions[1]
  t.eq(decision.outcome, fixture.decision_outcome, fixture.name .. ": exact guard disposition")
  local emitted_effects, observable_writes = effects_from_raises(result.raises)
  return {
    schema = "restart-old-behavior-observation.v2",
    observation_id = table.concat({
      "writer:github-devloop:observe-issue-unmanaged-thinking",
      fixture.name,
      "thinking",
      "guard-return",
      fixture.reason_code,
      "none",
    }, "/"),
    owner = "github-devloop",
    site = copy_value(SITE),
    boundary = "writer",
    typed_intent = {
      kind = "versioned_transition_status",
      source_state = JSON_NULL,
      source_boundary = event.queue,
      target = "thinking",
      cause_schema_id = event.payload.schema,
      generation_epoch = {
        current_version = nullable(decision.current.version),
        incoming_version = event.payload.dedup_key,
        target_version = JSON_NULL,
      },
      lineage = record_lineage(event.payload, event.payload.dedup_key, decision),
    },
    old_inputs = {
      current_fact = {
        state = nullable(decision.current.state),
        version = nullable(decision.current.version),
        stage_rank = nullable(decision.current.stage_rank),
      },
      caller_from_states = json_array({ "unmanaged" }),
      incoming_version = event.payload.dedup_key,
      target_version = JSON_NULL,
      handoff_reference = JSON_NULL,
    },
    old_outcome = {
      status = "guard-return",
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

local FIXTURES = {
  {
    name = "unsupported-payload",
    unsupported_payload = true,
    pre_cas = true,
    decision_outcome = "skip-foreign(proposal_id)",
    reason_code = "unsupported-event-payload",
  },
  {
    name = "issue-closed",
    labels = { "fkst-dev:enabled" },
    issue_state = "CLOSED",
    pre_cas = true,
    decision_outcome = "skip-advanced-or-diverged",
    reason_code = "issue-not-open",
  },
  {
    name = "not-opted-in",
    labels = {},
    pre_cas = true,
    decision_outcome = "skip-not-opted-in",
    reason_code = "enabled-label-absent",
  },
  {
    name = "intake-held",
    labels = { "fkst-dev:enabled", "fkst-dev:hold" },
    pre_cas = true,
    decision_outcome = "skip-held",
    reason_code = "hold-label-present",
  },
  {
    name = "unmanaged-foreign-assignee",
    labels = { "fkst-dev:enabled" },
    assignees = { "another-login" },
    decision_from_state = "claim",
    probe_outcome = "apply",
    decision_outcome = "skip-claimed-by-other",
    status = "guard-return",
    reason_code = "claim-held-by-other",
    effect_count = 0,
    expected_source_state = JSON_NULL,
    expected_source_boundary = "github-proxy.github_entity_changed",
  },
  {
    name = "unmanaged-author-unknown-at-admission",
    labels = { "fkst-dev:enabled" },
    assignees = json_array(),
    author_login = "",
    decision_from_state = "claim",
    probe_outcome = "apply",
    decision_outcome = "skip-fork-author-unknown",
    status = "guard-return",
    reason_code = "claim-author-unknown-at-admission",
    effect_count = 0,
    expected_source_state = JSON_NULL,
    expected_source_boundary = "github-proxy.github_entity_changed",
  },
  {
    name = "unmanaged-peer-bot-author",
    labels = { "fkst-dev:enabled" },
    assignees = json_array(),
    author_login = "peer-bot",
    managed_bot_logins = "fkst-test-bot,peer-bot",
    decision_from_state = "claim",
    probe_outcome = "apply",
    decision_outcome = "skip-fork-peer-bot",
    status = "guard-return",
    reason_code = "claim-peer-bot-author",
    effect_count = 0,
    expected_source_state = JSON_NULL,
    expected_source_boundary = "github-proxy.github_entity_changed",
  },
  {
    name = "unmanaged-unauthorized-author",
    labels = { "fkst-dev:enabled" },
    assignees = json_array(),
    author_login = "human",
    authorized_logins = "",
    decision_from_state = "claim",
    probe_outcome = "apply",
    decision_outcome = "skip-non-whitelisted-author",
    status = "guard-return",
    reason_code = "claim-author-not-authorized",
    effect_count = 0,
    expected_source_state = JSON_NULL,
    expected_source_boundary = "github-proxy.github_entity_changed",
  },
  {
    name = "unmanaged-existing-fork",
    labels = { "fkst-dev:enabled" },
    assignees = json_array(),
    author_login = "human",
    authorized_logins = "human",
    comments = function(event)
      return json_array({ fork_parent_comment(event.payload.repo, event.payload.number) })
    end,
    decision_from_state = "claim",
    probe_outcome = "apply",
    decision_outcome = "fork-present",
    status = "guard-return",
    reason_code = "fork-present-before-rederive",
    effect_count = 0,
    expected_source_state = JSON_NULL,
    expected_source_boundary = "github-proxy.github_entity_changed",
  },
  {
    name = "unmanaged-fork-grace",
    labels = { "fkst-dev:enabled" },
    assignees = json_array(),
    author_login = "human",
    authorized_logins = "human",
    created_at = T_NOT_TIMEOUT_DUE,
    decision_from_state = "claim",
    probe_outcome = "apply",
    decision_outcome = "skip-fork-grace",
    status = "guard-return",
    reason_code = "fork-grace-pending",
    effect_count = 0,
    expected_source_state = JSON_NULL,
    expected_source_boundary = "github-proxy.github_entity_changed",
  },
  {
    name = "unmanaged-fork-original-closed-after-rederive",
    labels = { "fkst-dev:enabled" },
    assignees = json_array(),
    author_login = "human",
    authorized_logins = "human",
    rederived = function()
      return { state = "CLOSED", author_login = "human" }
    end,
    decision_from_state = "claim",
    probe_outcome = "apply",
    decision_outcome = "skip-fork-original-closed",
    status = "guard-return",
    reason_code = "fork-original-closed-after-rederive",
    effect_count = 0,
    expected_source_state = JSON_NULL,
    expected_source_boundary = "github-proxy.github_entity_changed",
  },
  {
    name = "unmanaged-fork-author-missing-after-rederive",
    labels = { "fkst-dev:enabled" },
    assignees = json_array(),
    author_login = "human",
    authorized_logins = "human",
    rederived = function()
      return { state = "OPEN", author_login = "" }
    end,
    decision_from_state = "claim",
    probe_outcome = "apply",
    decision_outcome = "skip-fork-author-unknown",
    status = "guard-return",
    reason_code = "fork-author-missing-after-rederive",
    effect_count = 0,
    expected_source_state = JSON_NULL,
    expected_source_boundary = "github-proxy.github_entity_changed",
  },
  {
    name = "unmanaged-fork-appears-after-rederive",
    labels = { "fkst-dev:enabled" },
    assignees = json_array(),
    author_login = "human",
    authorized_logins = "human",
    rederived = function()
      return {
        state = "OPEN",
        author_login = "human",
        comments = json_array({ fork_parent_comment(REPO, ISSUE_NUMBER) }),
      }
    end,
    decision_from_state = "claim",
    probe_outcome = "apply",
    decision_outcome = "fork-present",
    status = "guard-return",
    reason_code = "fork-present-after-rederive",
    effect_count = 0,
    expected_source_state = JSON_NULL,
    expected_source_boundary = "github-proxy.github_entity_changed",
  },
  {
    name = "unmanaged-fork-raised",
    labels = { "fkst-dev:enabled" },
    assignees = json_array(),
    author_login = "human",
    authorized_logins = "human",
    rederived = function()
      return { state = "OPEN", author_login = "human" }
    end,
    decision_from_state = "claim",
    probe_outcome = "apply",
    decision_outcome = "fork-raised",
    status = "guard-return",
    reason_code = "fork-raised-before-management",
    effect_count = 1,
    expected_source_state = JSON_NULL,
    expected_source_boundary = "github-proxy.github_entity_changed",
  },
  {
    name = "unmanaged-label-claim-lost",
    issue_number = 42001,
    labels = { "fkst-dev:enabled" },
    assignees = json_array(),
    claim_mode = "label",
    write_mode = "1",
    claim_commands = function(event)
      local number = tostring(event.payload.number)
      t.mock_command("gh issue edit " .. number .. " --repo owner/repo --add-label fkst-dev:claimed", {
        stdout = "", stderr = "", exit_code = 0,
      })
      t.mock_command("gh issue view '" .. number .. "' --repo 'owner/repo' --json assignees,author,labels", {
        stdout = entity_read_mocks.issue_view_stdout({ number = event.payload.number, labels = { "fkst-dev:enabled" }, assignees = json_array() }),
        stderr = "", exit_code = 0,
      })
      t.mock_command("gh issue edit " .. number .. " --repo owner/repo --remove-label fkst-dev:claimed", {
        stdout = "", stderr = "", exit_code = 0,
      })
    end,
    decision_from_state = "claim",
    probe_outcome = "apply",
    decision_outcome = "claim-lost",
    status = "guard-return",
    reason_code = "label-claim-lost-after-verification",
    effect_count = 0,
    expected_source_state = JSON_NULL,
    expected_source_boundary = "github-proxy.github_entity_changed",
  },
  {
    name = "unmanaged-assignee-claim-permission-denied",
    issue_number = 42002,
    labels = { "fkst-dev:enabled" },
    assignees = json_array(),
    write_mode = "1",
    claim_commands = function(event)
      t.mock_command("gh issue edit " .. tostring(event.payload.number)
        .. " --repo owner/repo --add-assignee fkst-test-bot", {
        stdout = "",
        stderr = "GraphQL: Could not resolve to a User with the login of 'fkst-test-bot'. (permission-denied)\n",
        exit_code = 1,
      })
    end,
    decision_from_state = "claim",
    probe_outcome = "apply",
    decision_outcome = "skip-claim-permission-denied",
    status = "guard-return",
    reason_code = "assignee-claim-permission-denied",
    effect_count = 0,
    expected_source_state = JSON_NULL,
    expected_source_boundary = "github-proxy.github_entity_changed",
  },
  {
    name = "unmanaged-assignee-claim-lost",
    issue_number = 42003,
    labels = { "fkst-dev:enabled" },
    assignees = json_array(),
    write_mode = "1",
    claim_commands = function(event)
      local number = tostring(event.payload.number)
      t.mock_command("gh issue edit " .. number .. " --repo owner/repo --add-assignee fkst-test-bot", {
        stdout = "", stderr = "", exit_code = 0,
      })
      t.mock_command(core.gh_issue_view_claim_cmd(REPO, event.payload.number), {
        stdout = entity_read_mocks.issue_view_stdout({ number = event.payload.number, assignees = { "other-bot" } }),
        stderr = "", exit_code = 0,
      })
      t.mock_command("gh issue edit " .. number .. " --repo owner/repo --remove-assignee fkst-test-bot", {
        stdout = "", stderr = "", exit_code = 0,
      })
    end,
    decision_from_state = "claim",
    probe_outcome = "apply",
    decision_outcome = "claim-lost",
    status = "guard-return",
    reason_code = "assignee-claim-lost-after-verification",
    effect_count = 0,
    expected_source_state = JSON_NULL,
    expected_source_boundary = "github-proxy.github_entity_changed",
  },
  {
    name = "unmanaged-assignee-claim-error",
    issue_number = 42004,
    labels = { "fkst-dev:enabled" },
    assignees = json_array(),
    write_mode = "1",
    claim_commands = function(event)
      t.mock_command("gh issue edit " .. tostring(event.payload.number)
        .. " --repo owner/repo --add-assignee fkst-test-bot", {
        stdout = "", stderr = "HTTP 502: upstream unavailable\n", exit_code = 1,
      })
    end,
    probe_outcome = "apply",
    error_contains = "gh-command-failed",
    status = "error",
    reason_code = "assignee-claim-error-propagated",
    cas_outcome = "error(gh-command-failed)",
    effect_count = 0,
    expected_source_state = JSON_NULL,
    expected_source_boundary = "github-proxy.github_entity_changed",
  },
  {
    name = "unmanaged-ingress-apply",
    labels = { "fkst-dev:enabled" },
    needs_context = true,
    probe_outcome = "apply",
    decision_outcome = "applied",
    status = "apply",
    reason_code = "apply",
    effect_count = 3,
    applies_thinking = true,
    expected_source_state = JSON_NULL,
    expected_source_boundary = "github-proxy.github_entity_changed",
  },
  {
    name = "thinking-live-idempotent-reemit",
    labels = { "fkst-dev:enabled", "fkst-dev:thinking" },
    comments = function()
      return json_array({ state_comment("thinking", proposal_version(T_EQUAL)) })
    end,
    needs_context = true,
    live_thinking = true,
    excluded_decision_count = 1,
    probe_outcome = "idempotent",
    decision_outcome = "skip-idempotent(already at to_state)",
    status = "idempotent",
    reason_code = "already-thinking-reemit",
    effect_count = 3,
    applies_thinking = true,
    expected_source_state = "thinking",
    expected_source_boundary = JSON_NULL,
  },
  {
    name = "thinking-older-event",
    event_time = T_OLDER,
    labels = { "fkst-dev:enabled", "fkst-dev:thinking" },
    comments = function()
      return json_array({ state_comment("thinking", proposal_version(T_EQUAL)) })
    end,
    live_thinking = true,
    excluded_decision_count = 1,
    probe_outcome = "stale",
    decision_outcome = "skip-stale(incoming version < current marker version)",
    status = "stale",
    reason_code = "incoming-version-older",
    effect_count = 0,
    expected_source_state = "thinking",
    expected_source_boundary = JSON_NULL,
  },
  {
    name = "declined-state-advanced",
    labels = { "fkst-dev:enabled", "fkst-dev:declined" },
    comments = function()
      return json_array({ state_comment("declined", proposal_version(T_EQUAL)) })
    end,
    probe_outcome = "stale",
    decision_outcome = "skip-advanced-or-diverged",
    status = "stale",
    reason_code = "advanced-or-diverged",
    effect_count = 0,
    expected_source_state = "declined",
    expected_source_boundary = JSON_NULL,
  },
}

local function assert_pending_is_unreachable_for_unmanaged_intake()
  local states = json_array({ "unmanaged" })
  for state, _ in pairs(devloop_state.lifecycle_state_set()) do
    table.insert(states, state)
  end
  for _, state in ipairs(states) do
    local outcome = devloop_state.versioned_transition_status({
      state = state == "unmanaged" and nil or state,
      version = proposal_version(T_EQUAL),
    }, { "unmanaged" }, "thinking", proposal_version(T_EQUAL))
    t.is_true(outcome ~= "pending", "unmanaged->thinking pending is unreachable from " .. tostring(state))
  end
end

local function capture_records()
  assert_pending_is_unreachable_for_unmanaged_intake()
  local records = json_array()
  for _, fixture in ipairs(FIXTURES) do
    local ok, record = pcall(function()
      local event = issue_event(fixture.event_time, fixture.issue_number)
      if fixture.unsupported_payload then
        event.payload.type = "discussion"
      end
      local result, captured = observe_real_department(event, fixture)
      local scoped = site_capture(captured)
      return fixture.pre_cas
        and build_guard_record(event, result, scoped, fixture)
        or build_cas_record(event, result, scoped, fixture)
    end)
    if not ok then
      error("OLD observe_issue fixture " .. fixture.name .. " failed: " .. tostring(record), 0)
    end
    table.insert(records, record)
  end
  table.sort(records, function(left, right)
    return tostring(left.observation_id) < tostring(right.observation_id)
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
  test_observe_issue_old_observations_are_runtime_bound_and_deterministic = function()
    local first = capture_records()
    local second = capture_records()
    local repeat_difference = first_difference(second, first, "old_behavior_observations[observe-issue-repeat]")
    if repeat_difference ~= nil or canonical_json(second) ~= canonical_json(first) then
      error("second OLD observe_issue runtime capture differs at " .. tostring(repeat_difference or "canonical-json"), 0)
    end

    local expected = committed_records()
    local inventory_difference = first_difference(first, expected, "old_behavior_observations[observe-issue]")
    if inventory_difference ~= nil or canonical_json(first) ~= canonical_json(expected) then
      error(
        "runtime-bound OLD observe_issue observation differs at " .. tostring(inventory_difference or "canonical-json")
          .. "; runtime_records=" .. canonical_json(first),
        0
      )
    end
  end,
}
