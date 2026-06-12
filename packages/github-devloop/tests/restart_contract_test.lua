local h = require("tests.devloop_core_helpers")
local core = h.core
local t = h.t

local function has_value(values, expected)
  for _, value in ipairs(values or {}) do
    if value == expected then
      return true
    end
  end
  return false
end

local function copy_rows(rows)
  local copied = {}
  for index, row in ipairs(rows or {}) do
    local next_row = {}
    for key, value in pairs(row) do
      if type(value) == "table" then
        local nested = {}
        for nested_key, nested_value in pairs(value) do
          nested[nested_key] = nested_value
        end
        next_row[key] = nested
      else
        next_row[key] = value
      end
    end
    copied[index] = next_row
  end
  return copied
end

local function parse_marker_builders(paths)
  local families = {}
  for _, path in ipairs(paths) do
    local text = file.read(path)
    for family in text:gmatch("fkst:github%-devloop:([%w%-]+):v1") do
      families[family] = families[family] or {}
    end
    for family, attrs in pairs(families) do
      local family_pattern = "fkst:github%-devloop:" .. family:gsub("%-", "%%-") .. ":v1"
      local start_pos = text:find(family_pattern)
      if start_pos ~= nil then
        local function_pos = text:sub(1, start_pos):match("^.*()\nfunction M%.[^\n]+")
        local next_function = text:find("\nfunction M%.", start_pos + 1)
        local block = text:sub(function_pos or start_pos, next_function or #text)
        for attr in block:gmatch('" ([%w_]+)="') do
          attrs[attr] = true
        end
        for attr in block:gmatch('([%w_]+)="') do
          attrs[attr] = true
        end
      end
    end
  end
  return families
end

local function marker_builder_paths()
  return {
    "packages/github-devloop/core/state.lua",
    "packages/github-devloop/core/markers.lua",
    "packages/github-devloop/core/convergence.lua",
    "packages/github-devloop/core/dependencies.lua",
    "packages/github-devloop/core/decompose.lua",
  }
end

local function table_by_state()
  local by_state = {}
  for _, row in ipairs(core.restart_transition_table()) do
    by_state[row.from_state] = row
  end
  return by_state
end

local function allowed_extra_transition(state, next_state)
  return state == "reviewing" and next_state == "blocked"
end

return {
  test_persistence_class_is_declared = function()
    t.eq(core.persistence_class(), "saga")
  end,

  test_executable_restart_table_covers_non_terminal_states = function()
    local expected = {
      "thinking",
      "ready",
      "implementing",
      "pr-open",
      "reviewing",
      "merge-ready",
      "merging",
      "fixing",
      "review-meta",
      "blocked",
    }
    local by_state = table_by_state()
    for _, state in ipairs(expected) do
      local row = by_state[state]
      t.is_true(row ~= nil)
      t.eq(row.from_state, state)
      t.is_true(type(row.to_states) == "table")
      t.is_true(type(row.driving_queue) == "string" and row.driving_queue ~= "")
      t.is_true(type(row.payload_builder) == "function")
      t.is_true(type(row.dedup_shape) == "string" and row.dedup_shape ~= "")
      t.is_true(type(row.required_facts) == "table" and #row.required_facts > 0)
      t.is_true(type(row.payload_fields) == "table")
      t.is_true(type(row.version_identity) == "string" and row.version_identity ~= "")
      t.is_true(type(row.effects) == "table")
      t.is_true(tonumber(row.effects.intent_count) ~= nil)
      t.is_true(type(row.effects.completeness) == "string" and row.effects.completeness ~= "")
    end
    t.eq(#core.restart_transition_table(), #expected)
  end,

  test_restart_table_matches_state_graph_and_stage_rank = function()
    local by_state = table_by_state()
    local expected = {
      thinking = true,
      ready = true,
      implementing = true,
      ["pr-open"] = true,
      reviewing = true,
      ["merge-ready"] = true,
      merging = true,
      fixing = true,
      ["review-meta"] = true,
      blocked = true,
    }
    for state, next_states in pairs(core._state_graph) do
      if expected[state] then
        local row = by_state[state]
        t.is_true(row ~= nil)
        for _, next_state in ipairs(row.to_states) do
          t.is_true(has_value(next_states, next_state) or allowed_extra_transition(state, next_state))
        end
        t.is_true(core.stage_rank(state) > 0)
      end
    end
    for state in pairs(expected) do
      t.is_true(by_state[state] ~= nil)
    end
  end,

  test_restart_required_facts_declare_freshness_modes = function()
    for _, row in ipairs(core.restart_transition_table()) do
      local saw_marker = false
      for _, required in ipairs(row.required_facts) do
        t.is_true(type(required.family) == "string" and required.family ~= "")
        t.is_true(required.freshness == "marker-read" or required.freshness == "fetch-before-compare")
        if required.freshness == "marker-read" then
          saw_marker = true
        end
      end
      t.is_true(saw_marker)
    end
  end,

  test_restart_payload_fields_are_covered_by_durable_fields = function()
    local errors = core.restart_field_coverage_errors()
    t.eq(#errors, 0)
  end,

  test_restart_field_coverage_catches_374_shape_missing_gate_baseline = function()
    local rows = copy_rows(core.restart_transition_table())
    local marker_fields = core.restart_durable_marker_fields()
    marker_fields["merge-gate"].gate_baseline_sha = nil
    local errors = core.restart_field_coverage_errors(rows)
    marker_fields["merge-gate"].gate_baseline_sha = true
    t.eq(#errors, 1)
    t.is_true(errors[1]:find("fixing.gate_baseline_sha", 1, true) ~= nil)
    t.is_true(errors[1]:find("merge-gate.gate_baseline_sha", 1, true) ~= nil)
  end,

  test_declared_marker_fields_exist_in_marker_builders = function()
    local parsed = parse_marker_builders(marker_builder_paths())
    for family, attrs in pairs(core.restart_durable_marker_fields()) do
      t.is_true(parsed[family] ~= nil, "missing marker family " .. tostring(family))
      for attr in pairs(attrs) do
        t.is_true(parsed[family][attr] == true, "missing marker attr " .. tostring(family) .. "." .. tostring(attr))
      end
    end
  end,

  test_source_ref_derivations_are_declared = function()
    local derivations = core.restart_source_ref_derivations()
    t.eq(derivations.issue, true)
    t.eq(derivations.pr, true)
    t.eq(derivations.entity, true)
  end,
}
