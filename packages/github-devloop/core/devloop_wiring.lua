local W = {}
local registry = require("contract.registry")

local package_name = "github-devloop"

local function index_module(base)
  return base .. ".index"
end

local function issue_entry_name(index_entry)
  if type(index_entry) == "string" then
    return index_entry
  end
  return index_entry.module
end

local function load_entries(base, index)
  local entries = {}
  for _, index_entry in ipairs(index) do
    local name = issue_entry_name(index_entry)
    table.insert(entries, require(base .. "." .. name))
  end
  return entries
end

local function issue_registry_map(base, key_field, M)
  local index = require(index_module(base))
  local entries = load_entries(base, index)
  return registry.build_indexed_map(index_module(base), index, entries, key_field, M, nil, package_name)
end

function W.restart(M)
  local marker_fields = issue_registry_map("core.restart.marker_fields", "family", M)
  local replay_payload_fields = issue_registry_map("core.restart.required_replay_payload_fields", "state", M)
  local transitions_base = "core.restart.transitions"
  local transitions_index = require(index_module(transitions_base))
  local transitions = load_entries(transitions_base, transitions_index)
  return {
    marker_fields = marker_fields,
    replay_payload_fields = replay_payload_fields,
    transitions_index = transitions_index,
    transitions = transitions,
    transitions_label = index_module(transitions_base),
  }
end

function W.liveness(M)
  local producers = issue_registry_map("core.restart.liveness_signal_producers", "family", M)
  return {
    liveness_signal_producers = producers,
  }
end

function W.prompts()
  return {
    prompts = {
      fix = require("prompts.fix"),
      fix_reflection = require("prompts.fix_reflection"),
      implement = require("prompts.implement"),
      review_meta = require("prompts.review_meta"),
    },
  }
end

function W.gate_sources()
  return {
    child_start_visible = require("core.gates.child_start_visible"),
  }
end

return W
