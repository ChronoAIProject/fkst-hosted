local W = {}
local registry = require("workflow.registry")
local pr_review_replay_facts = require("devloop.restart.pr_review_replay_facts")

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
  local transitions_index = require("core.restart.transitions.index")
  local transitions = load_entries("core.restart.transitions", transitions_index)
  local replay_ops = pr_review_replay_facts.install(M)
  return {
    ops = replay_ops,
    marker_fields = marker_fields,
    replay_payload_fields = replay_payload_fields,
    transitions_index = transitions_index,
    transitions = transitions,
    transitions_label = "core.restart.transitions.index",
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
      implement = require("prompts.implement"),
    },
  }
end

function W.gate_sources()
  return {
    child_start_visible = require("core.gates.child_start_visible"),
  }
end

return W
