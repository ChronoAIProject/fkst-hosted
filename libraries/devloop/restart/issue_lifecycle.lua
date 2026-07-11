local C = {}

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

function C.transition_sources()
  local transitions_index = require(index_module(transitions_base))
  return {
    transitions_index = transitions_index,
    transitions = load_entries(transitions_base, transitions_index),
    transitions_label = index_module(transitions_base),
  }
end

return C
