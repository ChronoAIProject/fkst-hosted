local W = {}
local registry = require("workflow.registry")

local package_name = "github-devloop-pr"

local function index_module(base)
  return base .. ".index"
end

local function pr_entry_name(index_entry)
  if type(index_entry) == "string" then
    return index_entry
  end
  return index_entry.module
end

local function load_entries(base, index)
  local entries = {}
  for _, index_entry in ipairs(index) do
    local module_name = pr_entry_name(index_entry)
    table.insert(entries, require(base .. "." .. module_name))
  end
  return entries
end

local function pr_registry_map(base, key_field, M)
  local index = require(index_module(base))
  local entries = load_entries(base, index)
  return registry.build_indexed_map(index_module(base), index, entries, key_field, M, nil, package_name)
end

function W.restart(M)
  local transitions_base = "core.restart.transitions"
  local transitions_index = require(index_module(transitions_base))
  return {
    marker_fields = pr_registry_map("core.restart.marker_fields", "family", M),
    replay_payload_fields = pr_registry_map("core.restart.required_replay_payload_fields", "state", M),
    transitions_index = transitions_index,
    transitions = load_entries(transitions_base, transitions_index),
    transitions_label = index_module(transitions_base),
  }
end

function W.liveness(M)
  local producers = pr_registry_map("core.restart.liveness_signal_producers", "family", M)
  return {
    liveness_signal_producers = producers,
  }
end

function W.prompts()
  return {
    prompts = {
      fix = require("prompts.fix"),
      fix_reflection = require("prompts.fix_reflection"),
      review_meta = require("prompts.review_meta"),
    },
  }
end

return W
