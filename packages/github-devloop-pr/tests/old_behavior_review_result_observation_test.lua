local devloop_base = require("devloop.base")
local devloop_logging = require("devloop.logging")
local devloop_state = require("devloop.state")
local m_builders = require("devloop.markers.builders")
local payloads_predicates = require("devloop.payloads.predicates")
local testing = require("testkit.testing")
local transition_version = require("contract.transition_version")
local h = require("tests.devloop_helpers")
local review_result_department = require("departments.review_result.main")

local t = h.t
local core = h.core
local JSON_NULL = json.decode("null")
local JSON_ARRAY_TAG = getmetatable(json.decode("[]"))
local JSON_OBJECT_TAG = getmetatable(json.decode("{}"))
local INVENTORY_PATH = "migration/restart-lifecycle.inventory.json"
local SITE = {
  path = "packages/github-devloop-pr/departments/review_result/main.lua",
  symbol = "pipeline",
  ordinal = "cyclic_transition_status:reviewing->merge-ready|fixing|review-meta",
}

local V_OLDER = "2026-06-02T01-02-03Z"
local V_EQUAL = "2026-06-03T01-02-03Z"
local V_ORDERING_EQUAL_CURRENT = "v-loop-01"
local V_ORDERING_EQUAL_INCOMING = "v-loop-1"

local function nullable(value)
  if value == nil then
    return JSON_NULL
  end
  return value
end

local function json_string(value)
  return '"' .. tostring(value)
    :gsub("\\", "\\\\")
    :gsub('"', '\\"')
    :gsub("\b", "\\b")
    :gsub("\f", "\\f")
    :gsub("\n", "\\n")
    :gsub("\r", "\\r")
    :gsub("\t", "\\t")
    :gsub("[%z\1-\31]", function(char)
      return string.format("\\u%04x", string.byte(char))
    end)
    .. '"'
end

local function array_length(value)
  local count = 0
  local maximum = 0
  for key, _ in pairs(value) do
    if type(key) ~= "number" or key < 1 or key % 1 ~= 0 then
      return nil
    end
    count = count + 1
    if key > maximum then
      maximum = key
    end
  end
  if maximum ~= count then
    return nil
  end
  return maximum
end

local function json_array(values)
  local array = json.decode("[]")
  for index, value in ipairs(values or {}) do
    array[index] = value
  end
  return array
end

local function is_json_array(value)
  return type(value) == "table" and getmetatable(value) == JSON_ARRAY_TAG
end

local function json_container_kind(value)
  if is_json_array(value) then
    return "array"
  end
  local length = array_length(value)
  if length ~= nil and length > 0 then
    return "array"
  end
  return "object"
end

local function copy_value(value)
  if type(value) ~= "table" or value == JSON_NULL then
    return value
  end
  local length = array_length(value)
  local copy = {}
  if is_json_array(value) or (length ~= nil and length > 0) then
    copy = json_array()
  end
  for key, field in pairs(value) do
    copy[copy_value(key)] = copy_value(field)
  end
  return copy
end

local function canonical_json(value)
  if value == JSON_NULL then
    return "null"
  end
  local kind = type(value)
  if kind == "string" then
    return json_string(value)
  end
  if kind == "number" or kind == "boolean" then
    return tostring(value)
  end
  if kind ~= "table" then
    error("OLD observation canonical JSON cannot encode " .. kind)
  end

  if json_container_kind(value) == "array" then
    local length = array_length(value)
    if length == nil then
      error("OLD observation canonical JSON array must be a contiguous sequence")
    end
    local items = {}
    for index = 1, length do
      items[index] = canonical_json(value[index])
    end
    return "[" .. table.concat(items, ",") .. "]"
  end

  local keys = {}
  for key, _ in pairs(value) do
    if type(key) ~= "string" then
      error("OLD observation canonical JSON object key must be a string")
    end
    table.insert(keys, key)
  end
  table.sort(keys)
  local fields = {}
  for _, key in ipairs(keys) do
    table.insert(fields, json_string(key) .. ":" .. canonical_json(value[key]))
  end
  return "{" .. table.concat(fields, ",") .. "}"
end

local function first_difference(actual, expected, path)
  if actual == expected then
    return nil
  end
  if actual == JSON_NULL or expected == JSON_NULL then
    return path .. " (actual=" .. canonical_json(actual) .. ", expected=" .. canonical_json(expected) .. ")"
  end
  if type(actual) ~= type(expected) then
    return path .. " (actual type=" .. type(actual) .. ", expected type=" .. type(expected) .. ")"
  end
  if type(actual) ~= "table" then
    return path .. " (actual=" .. canonical_json(actual) .. ", expected=" .. canonical_json(expected) .. ")"
  end
  local actual_container = json_container_kind(actual)
  local expected_container = json_container_kind(expected)
  if actual_container ~= expected_container then
    return path .. " (actual container=" .. actual_container
      .. ", expected container=" .. expected_container .. ")"
  end
  local keys = {}
  for key, _ in pairs(actual) do keys[key] = true end
  for key, _ in pairs(expected) do keys[key] = true end
  local ordered = {}
  for key, _ in pairs(keys) do table.insert(ordered, key) end
  table.sort(ordered, function(left, right) return tostring(left) < tostring(right) end)
  for _, key in ipairs(ordered) do
    if actual[key] == nil then
      return path .. "." .. tostring(key) .. " (missing from runtime capture)"
    end
    if expected[key] == nil then
      return path .. "." .. tostring(key) .. " (missing from committed record)"
    end
    local difference = first_difference(actual[key], expected[key], path .. "." .. tostring(key))
    if difference ~= nil then
      return difference
    end
  end
  return nil
end

local function mock_branch_config()
  t.mock_command('printf %s "$FKST_DEVLOOP_UPSTREAM_BRANCH"', {
    stdout = "dev", stderr = "", exit_code = 0,
  })
  t.mock_command('printf %s "$FKST_DEVLOOP_INTEGRATION_BRANCH"', {
    stdout = "", stderr = "", exit_code = 0,
  })
end

local function review_event(fixture)
  local proposal_id = devloop_base.pr_review_proposal_id(
    "owner/repo", 7, fixture.incoming_version, "def456"
  )
  local decision = fixture.decision or "reject"
  return h.review_reached({
    proposal_id = proposal_id,
    dedup_key = "consensus:" .. proposal_id .. "/review",
    decision = decision,
    body = decision == "approve"
      and "Review consensus approves the diff."
      or "Review consensus rejects the diff.",
    blocking_gap = decision == "reject" and "missing OLD observation evidence" or nil,
  })
end

local function prepare_fixture(fixture, event)
  mock_branch_config()
  h.mock_default_issue_claim()
  local comments = json_array({
    m_builders.pr_origin_marker(
      "github-devloop/issue/owner/repo/42",
      "42",
      "devloop-owner-repo-42-01HY",
      fixture.incoming_version,
      "dev"
    ),
  })
  if fixture.current_state ~= nil then
    table.insert(comments, core.state_marker(
      "github-devloop/issue/owner/repo/42",
      fixture.current_state,
      fixture.current_version
    ))
    h.mock_pr_origin(comments, "devloop-owner-repo-42-01HY", "def456", "OPEN", "dev")
  else
    h.mock_pr_origin_for({
      comments = comments,
      head = "devloop-owner-repo-42-01HY",
      head_sha = "def456",
      state = "OPEN",
      base_branch = "dev",
    })
  end
  if event.decision == "approve" then
    h.mock_pr_normal_risk_diff_name_only()
  end
end

local function observe_real_department(run)
  local captured = {
    probes = json_array(),
    decisions = json_array(),
    applies = json_array(),
    raises = json_array(),
    handoff_direct_lookup_count = 0,
  }
  local original_cyclic = devloop_state.cyclic_transition_status
  local original_decision = devloop_logging.log_cas_decision
  local original_apply = devloop_logging.log_apply
  local original_raise = devloop_logging.log_raise
  local original_verified_handoff = payloads_predicates.verified_hand_off_state

  devloop_state.cyclic_transition_status = function(current, from_states, to_state, incoming_version, target_version)
    local outcome = original_cyclic(current, from_states, to_state, incoming_version, target_version)
    table.insert(captured.probes, {
      current = copy_value(current),
      from_states = copy_value(from_states),
      to_state = to_state,
      incoming_version = incoming_version,
      target_version = target_version,
      outcome = outcome,
    })
    return outcome
  end
  devloop_logging.log_cas_decision = function(dept, proposal_id, current, from_state, to_state, outcome, reason)
    if dept == "review_result" and from_state == "reviewing" then
      table.insert(captured.decisions, {
        dept = dept,
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
  devloop_logging.log_apply = function(dept, proposal_id, to_state, version, labels, queues)
    if dept == "review_result" then
      table.insert(captured.applies, {
        proposal_id = proposal_id,
        to_state = to_state,
        version = version,
        labels = copy_value(labels),
        queues = copy_value(queues),
      })
    end
    return original_apply(dept, proposal_id, to_state, version, labels, queues)
  end
  devloop_logging.log_raise = function(dept, proposal_id, queue, payload)
    if dept == "review_result" then
      table.insert(captured.raises, {
        proposal_id = proposal_id,
        queue = queue,
        payload = copy_value(payload),
      })
    end
    return original_raise(dept, proposal_id, queue, payload)
  end
  payloads_predicates.verified_hand_off_state = function(...)
    captured.handoff_direct_lookup_count = captured.handoff_direct_lookup_count + 1
    return original_verified_handoff(...)
  end

  local ok, result = pcall(run)
  payloads_predicates.verified_hand_off_state = original_verified_handoff
  devloop_logging.log_raise = original_raise
  devloop_logging.log_apply = original_apply
  devloop_logging.log_cas_decision = original_decision
  devloop_state.cyclic_transition_status = original_cyclic
  if not ok then
    error(result, 0)
  end
  return result, captured
end

local EFFECTS = {
  ["github-proxy.github_pr_comment_request"] = {
    effect_id = "comment:pr:review-result",
    sink_kind = "comment",
    authority_class = "lifecycle-authoritative",
  },
  ["github-proxy.github_issue_label_request"] = {
    effect_id = "label:issue:review-result",
    sink_kind = "label",
    authority_class = "lifecycle-authoritative",
  },
  ["devloop_review_meta"] = {
    effect_id = "queue:github-devloop-pr.devloop_review_meta",
    sink_kind = "queue",
    authority_class = "lifecycle-authoritative",
  },
}

local function outcome_status(probe, decision)
  if decision.outcome == "applied" then
    return "apply", "apply"
  end
  if decision.outcome == "skip-idempotent(already at to_state)" then
    return "idempotent", "already-at-target"
  end
  if decision.outcome == "retry-pending(from-state marker not yet visible)" then
    return "pending", "source-marker-not-visible"
  end
  if decision.outcome == "skip-stale(incoming version < current marker version)" then
    return "stale", "incoming-version-older"
  end
  if decision.outcome == "skip-stale(version-mismatch)" and probe.outcome == "apply" then
    return "stale", "version-mismatch"
  end
  error("unclassified OLD review_result outcome: " .. tostring(decision.outcome))
end

local function effects_from_raises(raises)
  local effects = json_array()
  local writes = json_array()
  for ordinal, raised in ipairs(raises) do
    local shape = EFFECTS[raised.queue]
    if shape == nil then
      error("unclassified OLD review_result raise: " .. tostring(raised.queue))
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

local function build_record(event, result, captured)
  t.eq(#captured.probes, 1, "real review_result CAS probe count")
  t.eq(#captured.decisions, 1, "real review_result CAS decision count")
  t.eq(#captured.raises, #result.raises, "logger and run_fake raise counts")
  for index, raised in ipairs(result.raises) do
    t.eq(canonical_json(captured.raises[index].payload), canonical_json(raised.payload), "captured raise payload " .. index)
    t.eq(captured.raises[index].queue, raised.queue, "captured raise queue " .. index)
  end

  local probe = captured.probes[1]
  local decision = captured.decisions[1]
  local status, reason_code = outcome_status(probe, decision)
  local emitted_effects, observable_writes = effects_from_raises(result.raises)
  local observation_id = table.concat({
    "writer:github-devloop-pr:review-result",
    tostring(probe.to_state),
    status,
    reason_code,
  }, "/")

  return {
    schema = "restart-old-behavior-observation.v2",
    observation_id = observation_id,
    owner = "github-devloop-pr",
    site = copy_value(SITE),
    boundary = "writer",
    typed_intent = {
      kind = "cyclic_transition_status",
      source_state = probe.from_states[1],
      source_boundary = JSON_NULL,
      target = probe.to_state,
      cause_schema_id = event.payload.schema,
      generation_epoch = {
        current_version = nullable(probe.current.version),
        incoming_version = probe.incoming_version,
        target_version = nullable(probe.target_version),
      },
      lineage = {
        proposal_id = decision.proposal_id,
        review_proposal_id = event.payload.proposal_id,
        dedup_key = event.payload.dedup_key,
        source_ref = copy_value(event.payload.source_ref),
      },
    },
    old_inputs = {
      current_fact = {
        state = nullable(probe.current.state),
        version = nullable(probe.current.version),
        stage_rank = nullable(probe.current.stage_rank),
      },
      caller_from_states = copy_value(probe.from_states),
      incoming_version = probe.incoming_version,
      target_version = nullable(probe.target_version),
      handoff_reference = JSON_NULL,
    },
    old_outcome = {
      status = status,
      reason_code = reason_code,
      cas_outcome = decision.outcome,
      emitted_effects = emitted_effects,
      observable_writes = observable_writes,
      handoff_direct_lookup_count = captured.handoff_direct_lookup_count,
      timeout_evidence_source = JSON_NULL,
    },
    evidence_refs = json_array({
      {
        kind = "runtime-cas-probe",
        ref = "devloop.state.cyclic_transition_status:" .. tostring(probe.outcome),
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
    decision = "approve",
    current_state = "reviewing",
    current_version = V_EQUAL,
    incoming_version = V_EQUAL,
    expected_target = "merge-ready",
    expected_status = "apply",
    expected_raise_count = 2,
  },
  {
    decision = "reject",
    current_state = "reviewing",
    current_version = V_EQUAL,
    incoming_version = V_EQUAL,
    expected_target = "fixing",
    expected_status = "apply",
    expected_raise_count = 2,
  },
  {
    decision = "reject",
    current_state = "reviewing",
    current_version = core.next_fix_version(core.next_fix_version(V_EQUAL)),
    incoming_version = core.next_fix_version(core.next_fix_version(V_EQUAL)),
    expected_target = "review-meta",
    expected_status = "apply",
    expected_raise_count = 2,
  },
  {
    decision = "approve",
    current_state = "merge-ready",
    current_version = V_EQUAL,
    incoming_version = V_EQUAL,
    expected_target = "merge-ready",
    expected_status = "idempotent",
    expected_raise_count = 0,
  },
  {
    decision = "approve",
    current_state = nil,
    current_version = nil,
    incoming_version = V_EQUAL,
    expected_target = "merge-ready",
    expected_status = "pending",
    expected_raise_count = 0,
    expected_failure = true,
  },
  {
    decision = "reject",
    current_state = "reviewing",
    current_version = V_EQUAL,
    incoming_version = V_OLDER,
    expected_target = "fixing",
    expected_status = "stale",
    expected_raise_count = 0,
  },
  {
    decision = "reject",
    current_state = "reviewing",
    current_version = V_ORDERING_EQUAL_CURRENT,
    incoming_version = V_ORDERING_EQUAL_INCOMING,
    expected_target = "fixing",
    expected_status = "stale",
    expected_raise_count = 0,
  },
}

local function capture_fixture(fixture)
  local event = {
    queue = "consensus.consensus_reached",
    payload = review_event(fixture),
  }
  prepare_fixture(fixture, event.payload)
  local result, captured = observe_real_department(function()
    if fixture.expected_failure then
      return testing.run_fake_expecting_failure(review_result_department, event)
    end
    return testing.run_fake(review_result_department, event)
  end)
  local record = build_record(event, result, captured)
  t.eq(record.typed_intent.target, fixture.expected_target, "route reaches expected target")
  t.eq(record.old_outcome.status, fixture.expected_status, "route reaches expected CAS regime")
  t.eq(#record.old_outcome.emitted_effects, fixture.expected_raise_count, "captured runtime effect count")
  t.eq(record.old_inputs.target_version, JSON_NULL, "four-argument cyclic call captures null target_version")
  t.eq(record.old_inputs.handoff_reference, JSON_NULL, "review_result consumes no handoff input")
  if fixture.expected_failure then
    t.is_true(result.failure ~= nil, "pending marker path fails for retry")
  end
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
  test_canonical_json_rejects_empty_array_object_drift = function()
    t.is_true(JSON_ARRAY_TAG ~= JSON_OBJECT_TAG, "json.decode preserves array versus object shape")

    local built = capture_fixture(FIXTURES[4])
    t.eq(canonical_json(built.old_outcome.emitted_effects), "[]", "built empty effects retain array shape")

    local drifted = copy_value(built)
    drifted.old_outcome.emitted_effects = {}
    local root = "old_behavior_observations[review_result][negative_control]"
    local difference = first_difference(drifted, built, root)
    t.is_true(
      canonical_json(drifted) ~= canonical_json(built),
      "empty array to empty object drift changes canonical JSON"
    )
    t.is_true(
      difference ~= nil
        and difference:find(root .. ".old_outcome.emitted_effects", 1, true) ~= nil,
      "empty-container drift diagnostic names old_outcome.emitted_effects: " .. tostring(difference)
    )
  end,

  test_review_result_old_observations_are_runtime_bound_to_inventory = function()
    t.is_true(
      transition_version.safe_version_segment(V_ORDERING_EQUAL_CURRENT)
        ~= transition_version.safe_version_segment(V_ORDERING_EQUAL_INCOMING),
      "safe-version mismatch fixture is byte-distinct"
    )
    t.eq(
      transition_version.compare(V_ORDERING_EQUAL_CURRENT, V_ORDERING_EQUAL_INCOMING),
      0,
      "safe-version mismatch fixture is ordering-equal"
    )

    local actual = json_array()
    for _, fixture in ipairs(FIXTURES) do
      table.insert(actual, capture_fixture(fixture))
    end
    table.sort(actual, function(left, right)
      return left.observation_id < right.observation_id
    end)

    local expected = committed_records()
    local difference = first_difference(actual, expected, "old_behavior_observations[review_result]")
    if difference ~= nil or canonical_json(actual) ~= canonical_json(expected) then
      error(
        "runtime-bound OLD review_result observation differs at " .. tostring(difference or "canonical-json")
          .. "; runtime_records=" .. canonical_json(actual),
        0
      )
    end
  end,
}
