local L = {}

local restart = require("devloop.restart")

local transitions_base = "devloop.restart.issue.transitions"

local function index_module(base)
  return base .. ".index"
end

local function entry_name(index_entry)
  if type(index_entry) == "string" then
    return index_entry
  end
  return index_entry.module
end

local function load_entries(base, index)
  local entries = {}
  for _, index_entry in ipairs(index) do
    table.insert(entries, require(base .. "." .. entry_name(index_entry)))
  end
  return entries
end

function L.transition_sources()
  local transitions_index = require(index_module(transitions_base))
  return {
    transitions_index = transitions_index,
    transitions = load_entries(transitions_base, transitions_index),
    transitions_label = index_module(transitions_base),
  }
end

function L.transition_table(M)
  return restart.transition_table(M, L.transition_sources())
end

local function lifecycle_row(row)
  return {
    from_state = row.from_state,
    terminal = row.terminal,
    driving_queue = row.driving_queue or "none",
    budget = row.budget,
  }
end

function L.lifecycle_rows(M)
  local rows = {}
  for _, row in ipairs(L.transition_table(M)) do
    table.insert(rows, lifecycle_row(row))
  end
  return rows
end

function L.install(M)
  local lifecycle_by_state = {}
  for _, row in ipairs(L.lifecycle_rows(M)) do
    lifecycle_by_state[row.from_state] = row
  end

  function M.lifecycle_transition_row(state_name)
    return lifecycle_by_state[state_name]
  end

  function M.liveness_budget_minutes(state_name)
    local row = M.lifecycle_transition_row(state_name)
    return row and row.budget and tonumber(row.budget.minutes) or nil
  end
end

return L
