local hidden_state = require("devloop.hidden_state_conformance")

local R = {}

local EVIDENCE_KIND = "production-replay/v1"

local function package_name(core)
  return tostring(core.restart_package_name or "github-devloop")
end

local function row_label(core, state)
  return package_name(core) .. "|" .. tostring(state or "?")
end

function R.production_replay(core, row)
  return hidden_state.hidden_state_row_recovery_errors(core, row)
end

function R.bind(state)
  return {
    state = state,
    evidence = EVIDENCE_KIND,
    run = R.production_replay,
  }
end

function R.inventory(states)
  local entries = {}
  for _, state in ipairs(states or {}) do
    table.insert(entries, R.bind(state))
  end
  return entries
end

function R.errors(core, rows, inventory)
  local messages = {}
  local nonterminal = {}
  for _, row in ipairs(rows or {}) do
    if row.terminal ~= true then
      nonterminal[tostring(row.from_state or "")] = row
    end
  end

  local seen = {}
  local executable = {}
  for index, entry in ipairs(inventory or {}) do
    local state = type(entry) == "table" and tostring(entry.state or "") or ""
    local label = row_label(core, state ~= "" and state or "entry-" .. tostring(index))
    if state == "" then
      table.insert(messages, label .. ": recovery evidence entry must name a state")
    elseif nonterminal[state] == nil then
      table.insert(messages, label .. ": recovery evidence names an unknown non-terminal restart row")
    elseif seen[state] == true then
      table.insert(messages, label .. ": duplicate recovery evidence entry")
    else
      seen[state] = true
      if entry.evidence ~= EVIDENCE_KIND or entry.run ~= R.production_replay then
        table.insert(messages, label .. ": recovery evidence entry has no executable production replay binding")
      else
        executable[state] = entry
      end
    end
  end

  for state, row in pairs(nonterminal) do
    local label = row_label(core, state)
    if seen[state] ~= true then
      table.insert(messages, label .. ": non-terminal restart row is missing recovery evidence")
    elseif executable[state] ~= nil then
      local ok, scenario_errors = pcall(executable[state].run, core, row)
      if not ok then
        table.insert(messages, label .. ": executable recovery evidence errored: " .. tostring(scenario_errors))
      elseif type(scenario_errors) ~= "table" then
        table.insert(messages, label .. ": executable recovery evidence returned no result table")
      else
        for _, message in ipairs(scenario_errors) do
          table.insert(messages, label .. ": executable recovery evidence failed: " .. tostring(message))
        end
      end
    end
  end

  table.sort(messages)
  return messages
end

return R
