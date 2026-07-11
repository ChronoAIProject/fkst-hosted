local h = require("tests.devloop_core_helpers")
local conformance = require("devloop.restart.issue_observation_conformance")
local facts = require("devloop.restart.issue_observation_facts")
local core = h.core
local t = h.t

local expected_states = {
  ["awaiting-pr"] = { from_state = "awaiting-pr", terminal = false, driving_queue = "devloop_observe_redrive", budget_minutes = 259200 },
  blocked = { from_state = "blocked", terminal = false, driving_queue = "github-devloop-decompose.devloop_decompose", budget_minutes = 1440 },
  declined = { from_state = "declined", terminal = true, driving_queue = "none", budget_minutes = nil },
  dependency_wait = { from_state = "dependency_wait", terminal = false, driving_queue = "devloop_observe_redrive", budget_minutes = 525600 },
  ["impl-failed"] = { from_state = "impl-failed", terminal = false, driving_queue = "devloop_ready", budget_minutes = 1440 },
  implementing = { from_state = "implementing", terminal = false, driving_queue = "devloop_ready", budget_minutes = 120 },
  merged = { from_state = "merged", terminal = true, driving_queue = "none", budget_minutes = nil },
  ready = { from_state = "ready", terminal = false, driving_queue = "devloop_ready", budget_minutes = 120 },
  thinking = { from_state = "thinking", terminal = false, driving_queue = "consensus.proposal", budget_minutes = 150 },
}

local function copy_value(value)
  if type(value) ~= "table" then
    return value
  end
  local copied = {}
  for key, field in pairs(value) do
    copied[key] = copy_value(field)
  end
  return copied
end

local function facts_copy()
  return {
    schema = facts.schema,
    owner = facts.owner,
    source_rows_fingerprint = facts.source_rows_fingerprint,
    states = copy_value(facts.states),
  }
end

local function mutation_errors(mutate)
  local changed = facts_copy()
  mutate(changed.states)
  changed.source_rows_fingerprint = conformance.source_rows_fingerprint(changed.states)
  return conformance.errors(core.restart_transition_table(), changed)
end

local function contains(errors, needle)
  for _, message in ipairs(errors or {}) do
    if tostring(message):find(needle, 1, true) ~= nil then
      return true
    end
  end
  return false
end

local function assert_rejected(errors, needle)
  t.is_true(#errors > 0, "expected observation facts conformance failure")
  t.is_true(contains(errors, needle), table.concat(errors, "\n"))
end

return {
  test_issue_observation_facts_publish_exact_closed_projection = function()
    t.eq(facts.schema, "restart-owner-observation-facts.v1")
    t.eq(facts.owner, "github-devloop")
    t.eq(facts.source_rows_fingerprint, conformance.source_rows_fingerprint(facts.states))

    local expected_seen = 0
    for state_name, expected in pairs(expected_states) do
      expected_seen = expected_seen + 1
      local state = facts.states[state_name]
      t.is_true(state ~= nil, state_name .. " missing")
      t.eq(state.from_state, expected.from_state)
      t.eq(state.terminal, expected.terminal)
      t.eq(state.driving_queue, expected.driving_queue)
      t.eq(state.budget_minutes, expected.budget_minutes)

      local row = facts.transition_row(state_name)
      t.eq(row.from_state, expected.from_state)
      t.eq(row.terminal, expected.terminal)
      t.eq(row.driving_queue, expected.driving_queue)
      t.eq(row.budget and row.budget.minutes or nil, expected.budget_minutes)
      t.eq(facts.budget_minutes(state_name), expected.budget_minutes)
    end
    t.eq(expected_seen, 9)

    local published_seen = 0
    for state_name, _ in pairs(facts.states) do
      published_seen = published_seen + 1
      t.is_true(expected_states[state_name] ~= nil, "unexpected published state " .. tostring(state_name))
    end
    t.eq(published_seen, 9)

    for _, state_name in ipairs({ "reviewing", "fixing", "merge-ready", "merging", "unknown-state" }) do
      t.is_nil(facts.transition_row(state_name), state_name)
      t.is_nil(facts.budget_minutes(state_name), state_name)
    end
  end,

  test_issue_observation_facts_match_assembled_owner_rows = function()
    local errors = conformance.errors(core.restart_transition_table())
    t.eq(#errors, 0, table.concat(errors, "\n"))
  end,

  test_issue_observation_facts_reject_added_state = function()
    assert_rejected(mutation_errors(function(states)
      states.extra = { from_state = "extra", terminal = false, driving_queue = "none" }
    end), "extra state extra")
  end,

  test_issue_observation_facts_reject_missing_state = function()
    assert_rejected(mutation_errors(function(states)
      states.ready = nil
    end), "missing state ready")
  end,

  test_issue_observation_facts_reject_changed_terminal = function()
    assert_rejected(mutation_errors(function(states)
      states.ready.terminal = true
    end), "state ready field terminal")
  end,

  test_issue_observation_facts_reject_changed_driving_queue = function()
    assert_rejected(mutation_errors(function(states)
      states.ready.driving_queue = "other_queue"
    end), "state ready field driving_queue")
  end,

  test_issue_observation_facts_reject_changed_budget = function()
    assert_rejected(mutation_errors(function(states)
      states.ready.budget_minutes = 121
    end), "state ready field budget_minutes")
  end,

  test_issue_observation_facts_reject_extra_state_field = function()
    local changed = facts_copy()
    changed.states.ready.extra = "unexpected"
    assert_rejected(conformance.errors(core.restart_transition_table(), changed), "state ready unexpected field extra")
  end,

  test_issue_observation_facts_reject_invalid_state_field_type = function()
    local changed = facts_copy()
    changed.states.ready.terminal = "false"
    assert_rejected(conformance.errors(core.restart_transition_table(), changed), "state ready field terminal must be boolean")
  end,

  test_issue_observation_facts_reject_false_override = function()
    assert_rejected(conformance.errors(core.restart_transition_table(), false), "must be a table")
  end,

  test_issue_observation_facts_reject_changed_fingerprint = function()
    local changed = facts_copy()
    changed.source_rows_fingerprint = "00000000"
    assert_rejected(conformance.errors(core.restart_transition_table(), changed), "source_rows_fingerprint")
  end,
}
