local canonicalization_inventory = require("core.restart.canonicalization_inventory")
local entry_inventory = require("core.restart.entry_inventory")
local h = require("tests.devloop_core_helpers")
local operator_reentry_inventory = require("core.restart.operator_reentry_inventory")
local restart_cas_catalog = require("devloop.restart_cas_catalog")
local restart_edges = require("devloop.restart_edges")

local core = h.core

local function starts_with(value, prefix)
  return value:sub(1, #prefix) == prefix
end

local function owner_for_id(id)
  if type(id) ~= "string" then
    error("binding/edge -> owner: id must be a string, got " .. type(id))
  end
  local normalized = id:gsub("^transition/", "")
  if starts_with(normalized, "github-devloop-pr/") then
    return "github-devloop-pr"
  end
  if starts_with(normalized, "github-devloop/") then
    return "github-devloop"
  end
  error("binding/edge -> owner: unknown owner for id " .. id)
end

local function pair_text(policy_id, variant)
  return "(" .. tostring(policy_id) .. ", " .. tostring(variant) .. ")"
end

local function same_pair(left_policy_id, left_variant, right_policy_id, right_variant)
  return left_policy_id == right_policy_id and left_variant == right_variant
end

local function count_keys(values)
  local count = 0
  for _ in pairs(values) do
    count = count + 1
  end
  return count
end

local function assert_count(actual, expected, message)
  if actual ~= expected then
    error(message .. ": expected " .. tostring(expected) .. ", got " .. tostring(actual))
  end
end

local function classify_bindings(owner)
  local real_bindings = {}
  local synthetic_bindings = {}
  local owner_binding_count = 0
  local seen_ids = {}

  for _, binding in ipairs(restart_cas_catalog.bindings()) do
    if owner_for_id(binding.id) == owner then
      owner_binding_count = owner_binding_count + 1
      if seen_ids[binding.id] then
        error("binding -> classification: duplicate binding id " .. binding.id)
      end
      seen_ids[binding.id] = true

      local classified = {
        id = binding.id,
        cas_policy_id = binding.cas_policy_id,
        variant = binding.variant,
      }
      if starts_with(binding.id, "transition/") then
        table.insert(synthetic_bindings, classified)
      else
        real_bindings[binding.id] = classified
      end
    end
  end

  assert_count(
    count_keys(real_bindings) + #synthetic_bindings,
    owner_binding_count,
    "binding -> classification: owner bindings were not exhaustively classified"
  )
  return real_bindings, synthetic_bindings, owner_binding_count
end

local function extracted_edge_groups(owner)
  local rows = core.restart_transition_table()
  local extractors = {
    autonomous = function()
      return restart_edges.extract_autonomous_edges(owner, rows)
    end,
    canonicalization = function()
      return restart_edges.extract_canonicalization_edges(owner, canonicalization_inventory)
    end,
    entry = function()
      return restart_edges.extract_entry_edges(owner, entry_inventory, rows)
    end,
    guard_boundary = function()
      return restart_edges.extract_guard_boundary_edges(owner, rows)
    end,
    operator_reentry = function()
      return restart_edges.extract_operator_reentry_edges(owner, operator_reentry_inventory)
    end,
    timeout = function()
      return restart_edges.extract_timeout_edges(owner, rows)
    end,
  }

  local groups = {}
  for kind, enabled in pairs(restart_edges.schema().extracted_kinds) do
    if enabled then
      if extractors[kind] == nil then
        error("edge -> extraction: missing completeness extractor for kind " .. tostring(kind))
      end
      table.insert(groups, extractors[kind]())
    end
  end
  return groups
end

local function collect_cas_edges(owner)
  local cas_edges = {}
  local seen_edge_ids = {}
  local cas_edge_count = 0

  for _, group in ipairs(extracted_edge_groups(owner)) do
    for _, edge in ipairs(group) do
      if owner_for_id(edge.id) == owner then
        if seen_edge_ids[edge.id] then
          error("edge -> uniqueness: duplicate owner edge id " .. edge.id)
        end
        seen_edge_ids[edge.id] = true
        if edge.cas_policy_id ~= nil then
          cas_edges[edge.id] = {
            id = edge.id,
            cas_policy_id = edge.cas_policy_id,
            cas_variant = edge.cas_variant,
          }
          cas_edge_count = cas_edge_count + 1
        end
      end
    end
  end

  assert_count(
    count_keys(cas_edges),
    cas_edge_count,
    "edge -> collection: CAS edges were not exhaustively keyed"
  )
  return cas_edges, cas_edge_count
end

local function assert_real_pair(binding, edge)
  if not same_pair(binding.cas_policy_id, binding.variant, edge.cas_policy_id, edge.cas_variant) then
    error(
      "binding -> edge: real binding CAS mismatch for " .. binding.id
        .. ": binding=" .. pair_text(binding.cas_policy_id, binding.variant)
        .. " edge=" .. pair_text(edge.cas_policy_id, edge.cas_variant)
    )
  end
end

local function synthetic_match_count(edge, real_bindings, synthetic_bindings)
  if real_bindings[edge.id] ~= nil then
    return 0
  end
  local matches = 0
  for _, binding in ipairs(synthetic_bindings) do
    if same_pair(binding.cas_policy_id, binding.variant, edge.cas_policy_id, edge.cas_variant) then
      matches = matches + 1
    end
  end
  return matches
end

return {
  test_restart_cas_bindings_correspond_to_all_inline_cas_edges = function()
    local owner = core.restart_package_name
    local real_bindings, synthetic_bindings, owner_binding_count = classify_bindings(owner)
    local cas_edges, cas_edge_count = collect_cas_edges(owner)
    local matched_real_edges = 0

    for id, binding in pairs(real_bindings) do
      local edge = cas_edges[id]
      if edge == nil then
        error("binding -> edge: real binding uncovered: " .. id)
      end
      assert_real_pair(binding, edge)
      matched_real_edges = matched_real_edges + 1
    end
    assert_count(
      matched_real_edges,
      count_keys(real_bindings),
      "binding -> edge: real bindings did not match one-to-one by id"
    )

    local matched_synthetic_bindings = 0
    for _, binding in ipairs(synthetic_bindings) do
      local matches = 0
      for _, edge in pairs(cas_edges) do
        if real_bindings[edge.id] == nil
            and same_pair(binding.cas_policy_id, binding.variant, edge.cas_policy_id, edge.cas_variant) then
          matches = matches + 1
        end
      end
      if matches == 0 then
        error(
          "binding -> edge: synthetic binding uncovered: " .. binding.id
            .. " CAS=" .. pair_text(binding.cas_policy_id, binding.variant)
        )
      end
      matched_synthetic_bindings = matched_synthetic_bindings + 1
    end
    assert_count(
      matched_synthetic_bindings,
      #synthetic_bindings,
      "binding -> edge: synthetic bindings were not exhaustively matched"
    )

    local matched_cas_edges = 0
    for id, edge in pairs(cas_edges) do
      local real_binding = real_bindings[id]
      local real_coverage = real_binding ~= nil and 1 or 0
      local synthetic_coverage = synthetic_match_count(edge, real_bindings, synthetic_bindings) > 0 and 1 or 0
      local coverage_categories = real_coverage + synthetic_coverage
      if coverage_categories == 0 then
        error(
          "edge -> binding: orphan CAS edge: " .. id
            .. " CAS=" .. pair_text(edge.cas_policy_id, edge.cas_variant)
        )
      end
      if coverage_categories ~= 1 then
        error("edge -> binding: CAS edge has ambiguous coverage direction: " .. id)
      end
      if real_binding ~= nil then
        assert_real_pair(real_binding, edge)
      end
      matched_cas_edges = matched_cas_edges + 1
    end
    assert_count(
      matched_cas_edges,
      cas_edge_count,
      "edge -> binding: CAS edges were not exhaustively matched"
    )
    assert_count(
      count_keys(real_bindings) + #synthetic_bindings,
      owner_binding_count,
      "binding -> edge: binding classification changed during correspondence proof"
    )
  end,
}
