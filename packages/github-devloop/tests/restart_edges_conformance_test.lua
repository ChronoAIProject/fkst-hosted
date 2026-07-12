local h = require("tests.devloop_core_helpers")
local entry_inventory = require("core.restart.entry_inventory")
local restart_cas_catalog = require("devloop.restart_cas_catalog")
local restart_edges = require("devloop.restart_edges")

local core = h.core
local t = h.t

local structural_fields = {
  "id",
  "owner",
  "row_id",
  "kind",
  "source",
  "target",
  "provenance",
}

local deferred_kinds = {}

local expected_successor_kinds = {
  ["awaiting-pr/child_pr_merged"] = "guard_boundary",
  ["awaiting-pr/child_pr_closed_unmerged_replaced"] = "guard_boundary",
  ["awaiting-pr/child_pr_not_merged"] = "guard_boundary",
  ["dependency_wait/blockers_still_open"] = "guard_boundary",
  ["dependency_wait/blockers_released"] = "guard_boundary",
  ["dependency_wait/dependency_resolver_stale"] = "guard_boundary",
  ["implementing/revision_published"] = "autonomous",
  ["implementing/revision_failed"] = "autonomous",
  ["ready/blocker_reappeared"] = "guard_boundary",
  ["ready/actionable_kickoff_timeout"] = "timeout",
  ["thinking/consensus-reached"] = "autonomous",
  ["thinking/consensus-reached-dependency-held"] = "autonomous",
  ["thinking/premise-refuted"] = "autonomous",
  ["thinking/consensus-stalled"] = "autonomous",
}

local expected_real_cas_by_id = {
  ["github-devloop/thinking/autonomous/consensus-reached"] = {
    cas_policy_id = "cas.legacy_consensus_result_v1",
    cas_variant = "thinking_to_ready",
  },
  ["github-devloop/thinking/autonomous/consensus-stalled"] = {
    cas_policy_id = "cas.legacy_loop_plain_v1",
    cas_variant = "thinking_to_blocked",
  },
}

local expected_guard_boundary_cas_by_id = {
  ["github-devloop/awaiting-pr/guard_boundary/child_pr_merged"] =
    { cas_policy_id = "cas.legacy_awaiting_pr_v1", cas_variant = "awaiting_pr_to_merged" },
  ["github-devloop/awaiting-pr/guard_boundary/child_pr_closed_unmerged_replaced"] =
    { cas_policy_id = "cas.legacy_awaiting_pr_v1", cas_variant = "awaiting_pr_to_ready" },
  ["github-devloop/awaiting-pr/guard_boundary/child_pr_not_merged"] =
    { cas_policy_id = "cas.legacy_awaiting_pr_v1", cas_variant = "awaiting_pr_to_blocked" },
}

local function key_set(keys)
  local out = {}
  for _, key in ipairs(keys) do
    out[key] = true
  end
  return out
end

local function assert_exact_keys(value, expected)
  local count = 0
  for key in pairs(value) do
    count = count + 1
    t.eq(expected[key], true)
  end
  local expected_count = 0
  for _ in pairs(expected) do
    expected_count = expected_count + 1
  end
  t.eq(count, expected_count)
end

local function copy_value(value)
  if type(value) ~= "table" then
    return value
  end
  local out = {}
  for key, nested in pairs(value) do
    out[key] = copy_value(nested)
  end
  return out
end

local function assert_same_value(actual, expected)
  if type(expected) ~= "table" then
    t.eq(actual, expected)
    return
  end
  t.eq(type(actual), "table")
  local actual_count = 0
  for _ in pairs(actual) do
    actual_count = actual_count + 1
  end
  local expected_count = 0
  for key, nested in pairs(expected) do
    expected_count = expected_count + 1
    assert_same_value(actual[key], nested)
  end
  t.eq(actual_count, expected_count)
end

local function binding_by_id(binding_id)
  for _, binding in ipairs(restart_cas_catalog.bindings()) do
    if binding.id == binding_id then
      return binding
    end
  end
  return nil
end

local function expected_edges(owner, rows)
  local expected = {}
  local empty_rows = {}
  for _, row in ipairs(rows) do
    local successors = row.responsibility_signature.successors
    local row_has_edge = false
    for _, successor in ipairs(successors) do
      if successor.kind == "autonomous" then
        row_has_edge = true
        table.insert(expected, {
          id = owner .. "/" .. row.from_state .. "/autonomous/" .. successor.output_variant,
          owner = owner,
          row_id = row.from_state,
          kind = "autonomous",
          source = { state = row.from_state, boundary = nil },
          target = successor.state,
          provenance = {
            owner = owner,
            row = row.from_state,
            field = "responsibility_signature.successors",
          },
        })
      end
    end
    if not row_has_edge then
      table.insert(empty_rows, row.from_state)
    end
  end
  return expected, empty_rows
end

local function expected_guard_boundary_edges(owner, rows)
  local expected = {}
  local rows_without_boundaries = {}
  for _, row in ipairs(rows) do
    local row_has_edge = false
    local signature = row.responsibility_signature
    for _, successor in ipairs(type(signature) == "table" and signature.successors or {}) do
      if successor.kind == "guard_boundary" then
        row_has_edge = true
        table.insert(expected, {
          id = owner .. "/" .. row.from_state .. "/guard_boundary/" .. successor.output_variant,
          owner = owner,
          row_id = row.from_state,
          kind = "guard_boundary",
          source = { state = row.from_state, boundary = nil },
          target = successor.state,
          provenance = {
            owner = owner,
            row = row.from_state,
            field = "responsibility_signature.successors",
          },
        })
        local expected_cas = expected_guard_boundary_cas_by_id[expected[#expected].id]
        if expected_cas ~= nil then
          expected[#expected].cas_policy_id = expected_cas.cas_policy_id
          expected[#expected].cas_variant = expected_cas.cas_variant
        end
      end
    end
    if row.guard_boundaries ~= nil then
      for _, guard_boundary in ipairs(row.guard_boundaries) do
        for _, successor in ipairs(guard_boundary.successors) do
          if successor.kind ~= "timeout" then
            row_has_edge = true
            table.insert(expected, {
              id = owner .. "/" .. row.from_state .. "/guard_boundary/" .. guard_boundary.name .. "/" .. successor.output_variant,
              owner = owner,
              row_id = row.from_state,
              kind = "guard_boundary",
              source = { state = row.from_state, boundary = guard_boundary.name },
              target = successor.state,
              provenance = {
                owner = owner,
                row = row.from_state,
                field = "guard_boundaries",
              },
            })
          end
        end
      end
    end
    if not row_has_edge then
      table.insert(rows_without_boundaries, row.from_state)
    end
  end
  return expected, rows_without_boundaries
end

local timeout_policy_by_source = {
  ["state_entry:v1"] = "timeout.state_entry_legacy_v1",
  ["codex_run:v1"] = "timeout.codex_run_legacy_v1",
  ["live_defer_heartbeat:v1"] = "timeout.heartbeat_legacy_v1",
  ["live_defer_epoch:v1"] = "timeout.durable_clear_legacy_v1",
  ["child_workflow_wait:v1"] = "timeout.child_workflow_legacy_v1",
}

local function expected_timeout_edges(owner, rows)
  local expected = {}
  for _, row in ipairs(rows) do
    local policy_id = timeout_policy_by_source[row.actionable_epoch and row.actionable_epoch.source]
    for _, successor in ipairs(row.responsibility_signature.successors) do
      if successor.kind == "timeout" then
        table.insert(expected, {
          id = owner .. "/" .. row.from_state .. "/timeout/" .. successor.output_variant,
          owner = owner,
          row_id = row.from_state,
          kind = "timeout",
          source = { state = row.from_state, boundary = nil },
          target = successor.state,
          timeout_evidence_policy_id = policy_id,
          provenance = {
            owner = owner,
            row = row.from_state,
            field = "responsibility_signature.successors",
          },
        })
      end
    end
    for _, guard_boundary in ipairs(row.guard_boundaries or {}) do
      for _, successor in ipairs(guard_boundary.successors) do
        if successor.kind == "timeout" then
          table.insert(expected, {
            id = owner .. "/" .. row.from_state .. "/timeout/" .. guard_boundary.name .. "/" .. successor.output_variant,
            owner = owner,
            row_id = row.from_state,
            kind = "timeout",
            source = { state = row.from_state, boundary = guard_boundary.name },
            target = successor.state,
            timeout_evidence_policy_id = policy_id,
            provenance = {
              owner = owner,
              row = row.from_state,
              field = "guard_boundaries",
            },
          })
        end
      end
    end
  end
  return expected
end

local function assert_edges(actual, expected, empty_rows)
  t.eq(#actual, #expected)
  local seen_ids = {}
  local seen_edges = {}
  local seen_sources = {}
  local seen_provenance = {}
  local counts_by_row = {}
  for index, expected_edge in ipairs(expected) do
    local edge = actual[index]
    local edge_keys = key_set(structural_fields)
    if expected_edge.cas_policy_id ~= nil then
      edge_keys.cas_policy_id = true
    end
    if expected_edge.cas_variant ~= nil then
      edge_keys.cas_variant = true
    end
    assert_exact_keys(edge, edge_keys)
    assert_exact_keys(edge.source, { state = true })
    assert_exact_keys(edge.provenance, { owner = true, row = true, field = true })
    t.eq(edge.id, expected_edge.id)
    t.eq(edge.owner, expected_edge.owner)
    t.eq(edge.row_id, expected_edge.row_id)
    t.eq(edge.kind, expected_edge.kind)
    t.eq(edge.source.state, expected_edge.source.state)
    t.eq(edge.source.boundary, nil)
    t.eq(edge.target, expected_edge.target)
    t.eq(edge.cas_policy_id, expected_edge.cas_policy_id)
    t.eq(edge.cas_variant, expected_edge.cas_variant)
    if expected_edge.binding_id ~= nil then
      local binding = binding_by_id(expected_edge.binding_id)
      t.is_true(binding ~= nil)
      t.eq(edge.cas_policy_id, binding.cas_policy_id)
      t.eq(edge.cas_variant, binding.variant)
    end
    t.eq(edge.provenance.owner, expected_edge.provenance.owner)
    t.eq(edge.provenance.row, expected_edge.provenance.row)
    t.eq(edge.provenance.field, expected_edge.provenance.field)
    t.eq(seen_ids[edge.id], nil)
    t.eq(seen_edges[edge], nil)
    t.eq(seen_sources[edge.source], nil)
    t.eq(seen_provenance[edge.provenance], nil)
    seen_ids[edge.id] = true
    seen_edges[edge] = true
    seen_sources[edge.source] = true
    seen_provenance[edge.provenance] = true
    counts_by_row[edge.row_id] = (counts_by_row[edge.row_id] or 0) + 1
  end
  for _, row_id in ipairs(empty_rows) do
    t.eq(counts_by_row[row_id] or 0, 0)
  end
end

local function assert_guard_boundary_edges(actual, expected, rows_without_boundaries)
  t.eq(#actual, #expected)
  local seen_ids = {}
  local seen_edges = {}
  local seen_sources = {}
  local seen_provenance = {}
  local counts_by_row = {}
  for index, expected_edge in ipairs(expected) do
    local edge = actual[index]
    local edge_keys = key_set(structural_fields)
    if expected_edge.cas_policy_id ~= nil then edge_keys.cas_policy_id = true end
    if expected_edge.cas_variant ~= nil then edge_keys.cas_variant = true end
    assert_exact_keys(edge, edge_keys)
    if expected_edge.source.boundary == nil then
      assert_exact_keys(edge.source, { state = true })
    else
      assert_exact_keys(edge.source, { state = true, boundary = true })
    end
    assert_exact_keys(edge.provenance, { owner = true, row = true, field = true })
    t.eq(edge.id, expected_edge.id)
    t.eq(edge.owner, expected_edge.owner)
    t.eq(edge.row_id, expected_edge.row_id)
    t.eq(edge.kind, expected_edge.kind)
    t.eq(edge.source.state, expected_edge.source.state)
    t.eq(edge.source.boundary, expected_edge.source.boundary)
    t.eq(edge.target, expected_edge.target)
    t.eq(edge.cas_policy_id, expected_edge.cas_policy_id)
    t.eq(edge.cas_variant, expected_edge.cas_variant)
    t.eq(edge.provenance.owner, expected_edge.provenance.owner)
    t.eq(edge.provenance.row, expected_edge.provenance.row)
    t.eq(edge.provenance.field, expected_edge.provenance.field)
    t.eq(seen_ids[edge.id], nil)
    t.eq(seen_edges[edge], nil)
    t.eq(seen_sources[edge.source], nil)
    t.eq(seen_provenance[edge.provenance], nil)
    seen_ids[edge.id] = true
    seen_edges[edge] = true
    seen_sources[edge.source] = true
    seen_provenance[edge.provenance] = true
    counts_by_row[edge.row_id] = (counts_by_row[edge.row_id] or 0) + 1
  end
  for _, row_id in ipairs(rows_without_boundaries) do
    t.eq(counts_by_row[row_id] or 0, 0)
  end
end

local function assert_timeout_edges(actual, expected)
  t.eq(#actual, #expected)
  local edge_keys = key_set(structural_fields)
  edge_keys.timeout_evidence_policy_id = true
  local seen_ids = {}
  for index, expected_edge in ipairs(expected) do
    local edge = actual[index]
    assert_exact_keys(edge, edge_keys)
    if expected_edge.source.boundary == nil then
      assert_exact_keys(edge.source, { state = true })
    else
      assert_exact_keys(edge.source, { state = true, boundary = true })
    end
    assert_exact_keys(edge.provenance, { owner = true, row = true, field = true })
    t.eq(edge.id, expected_edge.id)
    t.eq(edge.owner, expected_edge.owner)
    t.eq(edge.row_id, expected_edge.row_id)
    t.eq(edge.kind, "timeout")
    t.eq(edge.source.state, expected_edge.source.state)
    t.eq(edge.source.boundary, expected_edge.source.boundary)
    t.eq(edge.target, expected_edge.target)
    t.eq(edge.timeout_evidence_policy_id, expected_edge.timeout_evidence_policy_id)
    t.eq(edge.provenance.owner, expected_edge.provenance.owner)
    t.eq(edge.provenance.row, expected_edge.provenance.row)
    t.eq(edge.provenance.field, expected_edge.provenance.field)
    t.eq(seen_ids[edge.id], nil)
    seen_ids[edge.id] = true
  end
end

local function assert_successor_kind_partition(rows)
  local seen = {}
  for _, row in ipairs(rows) do
    for _, successor in ipairs(row.responsibility_signature.successors) do
      local key = row.from_state .. "/" .. tostring(successor.output_variant)
      t.eq(successor.kind, expected_successor_kinds[key])
      t.eq(seen[key], nil)
      seen[key] = true
    end
  end
  local expected_count = 0
  for key in pairs(expected_successor_kinds) do
    expected_count = expected_count + 1
    t.eq(seen[key], true)
  end
  local actual_count = 0
  for _ in pairs(seen) do
    actual_count = actual_count + 1
  end
  t.eq(actual_count, expected_count)
end

local function tuple_key(owner, source_state, target, output_variant)
  local state_key = source_state == nil and "\0" or "\1" .. source_state
  return table.concat({ owner, state_key, target, output_variant }, "\2")
end

local function output_variant_from_id(id)
  local output_variant = tostring(id):match("/([^/]+)$")
  t.is_true(output_variant ~= nil)
  return output_variant
end

local function sorted_set_bytes(values)
  local keys = {}
  for key in pairs(values) do
    table.insert(keys, key)
  end
  table.sort(keys)
  return table.concat(keys, "\n")
end

local function legacy_union_bytes(owner, rows, inventory)
  local tuples = {}
  for _, row in ipairs(rows) do
    for _, successor in ipairs(row.responsibility_signature.successors) do
      tuples[tuple_key(owner, row.from_state, successor.state, successor.output_variant)] = true
    end
    for _, guard_boundary in ipairs(row.guard_boundaries or {}) do
      for _, successor in ipairs(guard_boundary.successors) do
        tuples[tuple_key(owner, row.from_state, successor.state, successor.output_variant)] = true
      end
    end
    for _, activation in ipairs(row.receiver_activations or {}) do
      tuples[tuple_key(owner, row.from_state, activation.target, activation.output_variant)] = true
    end
  end
  for _, entry in ipairs(inventory) do
    tuples[tuple_key(owner, nil, entry.target, output_variant_from_id(entry.id))] = true
  end
  return sorted_set_bytes(tuples)
end

local function extracted_union_bytes(owner, rows, inventory)
  local tuples = {}
  local extracted = {
    restart_edges.extract_autonomous_edges(owner, rows),
    restart_edges.extract_guard_boundary_edges(owner, rows),
    restart_edges.extract_timeout_edges(owner, rows),
    restart_edges.extract_entry_edges(owner, inventory, rows),
  }
  for _, edges in ipairs(extracted) do
    for _, edge in ipairs(edges) do
      tuples[tuple_key(edge.owner, edge.source.state, edge.target, output_variant_from_id(edge.id))] = true
    end
  end
  return sorted_set_bytes(tuples)
end

local function row(from_state, successors)
  return {
    from_state = from_state,
    responsibility_signature = { successors = successors },
  }
end

local function guard_row(from_state, guard_boundaries)
  return {
    from_state = from_state,
    guard_boundaries = guard_boundaries,
  }
end

local function assert_extract_fails(owner, rows)
  local ok = pcall(function()
    restart_edges.extract_autonomous_edges(owner, rows)
  end)
  t.eq(ok, false)
end

local function assert_guard_extract_fails(owner, rows)
  local ok = pcall(function()
    restart_edges.extract_guard_boundary_edges(owner, rows)
  end)
  t.eq(ok, false)
end

local function assert_timeout_extract_fails(owner, rows)
  local ok = pcall(function()
    restart_edges.extract_timeout_edges(owner, rows)
  end)
  t.eq(ok, false)
end

local function row_by_state(rows, state)
  for _, candidate in ipairs(rows) do
    if candidate.from_state == state then
      return candidate
    end
  end
  return nil
end

return {
  test_restart_edges_schema_is_explicit_about_extracted_and_deferred_kinds = function()
    assert_exact_keys(restart_edges, {
      extract_autonomous_edges = true,
      extract_canonicalization_edges = true,
      extract_entry_edges = true,
      extract_guard_boundary_edges = true,
      extract_operator_reentry_edges = true,
      extract_timeout_edges = true,
      schema = true,
    })
    local schema = restart_edges.schema()
    assert_exact_keys(schema, {
      structural_fields = true,
      extracted_kinds = true,
      deferred_kinds = true,
    })
    t.eq(#schema.structural_fields, #structural_fields)
    for index, field in ipairs(structural_fields) do
      t.eq(schema.structural_fields[index], field)
    end
    assert_exact_keys(schema.extracted_kinds, {
      autonomous = true,
      canonicalization = true,
      entry = true,
      guard_boundary = true,
      operator_reentry = true,
      timeout = true,
    })
    t.eq(schema.extracted_kinds.autonomous, true)
    t.eq(schema.extracted_kinds.canonicalization, true)
    t.eq(schema.extracted_kinds.entry, true)
    t.eq(schema.extracted_kinds.guard_boundary, true)
    t.eq(schema.extracted_kinds.operator_reentry, true)
    t.eq(schema.extracted_kinds.timeout, true)
    t.eq(#schema.deferred_kinds, #deferred_kinds)
    for index, kind in ipairs(deferred_kinds) do
      t.eq(schema.deferred_kinds[index], kind)
    end
  end,

  test_restart_edges_match_issue_rows_in_registry_and_authored_successor_order = function()
    local owner = core.restart_package_name
    local rows = core.restart_transition_table()
    local snapshot = copy_value(rows)
    assert_successor_kind_partition(rows)
    local expected, empty_rows = expected_edges(owner, rows)
    for _, edge in ipairs(expected) do
      if edge.id == owner .. "/thinking/autonomous/consensus-reached-dependency-held" then
        edge.cas_policy_id = "cas.legacy_consensus_result_v1"
        edge.cas_variant = "thinking_to_dependency_wait"
      end
    end
    for _, edge in ipairs(expected) do
      local expected_cas = expected_real_cas_by_id[edge.id]
      if expected_cas ~= nil then
        edge.cas_policy_id = expected_cas.cas_policy_id
        edge.cas_variant = expected_cas.cas_variant
        edge.binding_id = edge.id
      end
    end
    local actual = restart_edges.extract_autonomous_edges(owner, rows)
    t.eq(#actual, 6)
    assert_edges(actual, expected, empty_rows)
    assert_same_value(rows, snapshot)

    local repeated = restart_edges.extract_autonomous_edges(owner, rows)
    assert_edges(repeated, expected, empty_rows)
    for index, edge in ipairs(actual) do
      t.is_true(edge ~= repeated[index])
      t.is_true(edge.source ~= repeated[index].source)
      t.is_true(edge.provenance ~= repeated[index].provenance)
    end

    -- Inventory-authored canonicalization edges have their own production-observation conformance.
    -- This successor conformance remains scoped to responsibility_signature.successors.
  end,

  test_thinking_dependency_wait_edge_cas_metadata_shadows_binding_647 = function()
    local owner = core.restart_package_name
    local edge_id = owner .. "/thinking/autonomous/consensus-reached-dependency-held"
    local edge
    for _, candidate in ipairs(restart_edges.extract_autonomous_edges(owner, core.restart_transition_table())) do
      if candidate.id == edge_id then
        edge = candidate
      end
    end

    local binding
    for _, candidate in ipairs(restart_cas_catalog.bindings()) do
      if candidate.id == "transition/github-devloop/consensus_result/thinking-dependency_wait" then
        binding = candidate
      end
    end

    t.is_true(edge ~= nil)
    t.is_true(binding ~= nil)
    t.eq(edge.cas_policy_id, "cas.legacy_consensus_result_v1")
    t.eq(edge.cas_variant, "thinking_to_dependency_wait")
    t.eq(edge.cas_policy_id, binding.cas_policy_id)
    t.eq(edge.cas_variant, binding.variant)
  end,

  test_autonomous_cas_metadata_is_optional_copied_and_fail_closed = function()
    local valid = row("from", {
      {
        state = "with-cas",
        output_variant = "with-cas",
        kind = "autonomous",
        cas_policy_id = "cas.synthetic_v1",
        cas_variant = "synthetic_variant",
      },
      { state = "without-cas", output_variant = "without-cas", kind = "autonomous" },
    })
    local edges = restart_edges.extract_autonomous_edges("owner", { valid })
    t.eq(#edges, 2)
    assert_exact_keys(edges[1], {
      id = true,
      owner = true,
      row_id = true,
      kind = true,
      source = true,
      target = true,
      cas_policy_id = true,
      cas_variant = true,
      provenance = true,
    })
    t.eq(edges[1].cas_policy_id, "cas.synthetic_v1")
    t.eq(edges[1].cas_variant, "synthetic_variant")
    assert_exact_keys(edges[2], key_set(structural_fields))
    t.eq(edges[2].cas_policy_id, nil)
    t.eq(edges[2].cas_variant, nil)

    assert_extract_fails("owner", {
      row("from", { { state = "to", output_variant = "done", kind = "autonomous", cas_policy_id = "" } }),
    })
    assert_extract_fails("owner", {
      row("from", { { state = "to", output_variant = "done", kind = "autonomous", cas_policy_id = 1 } }),
    })
    assert_extract_fails("owner", {
      row("from", { { state = "to", output_variant = "done", kind = "autonomous", cas_variant = "" } }),
    })
    assert_extract_fails("owner", {
      row("from", { { state = "to", output_variant = "done", kind = "autonomous", cas_variant = false } }),
    })
  end,

  test_restart_guard_boundary_edges_match_issue_rows_in_authored_order = function()
    local owner = core.restart_package_name
    local rows = core.restart_transition_table()
    local snapshot = copy_value(rows)
    local expected, rows_without_boundaries = expected_guard_boundary_edges(owner, rows)
    local actual = restart_edges.extract_guard_boundary_edges(owner, rows)
    t.eq(#expected, 7)
    assert_guard_boundary_edges(actual, expected, rows_without_boundaries)
    assert_same_value(rows, snapshot)

    local autonomous_only = row("autonomous-only", {
      { state = "autonomous-target", output_variant = "autonomous-output", kind = "autonomous" },
    })
    t.eq(#restart_edges.extract_guard_boundary_edges(owner, { autonomous_only }), 0)
  end,

  test_restart_timeout_edges_match_issue_rows_and_closed_evidence_policy = function()
    local owner = core.restart_package_name
    local rows = core.restart_transition_table()
    local snapshot = copy_value(rows)
    local expected = expected_timeout_edges(owner, rows)
    local actual = restart_edges.extract_timeout_edges(owner, rows)

    t.eq(#expected, 1)
    assert_timeout_edges(actual, expected)
    t.eq(actual[1].id, owner .. "/ready/timeout/actionable_kickoff_timeout")
    t.eq(actual[1].timeout_evidence_policy_id, "timeout.state_entry_legacy_v1")
    assert_same_value(rows, snapshot)

    local repeated = restart_edges.extract_timeout_edges(owner, rows)
    assert_timeout_edges(repeated, expected)
    t.is_true(actual[1] ~= repeated[1])
    t.is_true(actual[1].source ~= repeated[1].source)
    t.is_true(actual[1].provenance ~= repeated[1].provenance)
  end,

  test_restart_edge_kind_partition_preserves_the_legacy_issue_union_byte_for_byte = function()
    local owner = core.restart_package_name
    local rows = core.restart_transition_table()
    local before = legacy_union_bytes(owner, rows, entry_inventory)
    local after = extracted_union_bytes(owner, rows, entry_inventory)
    t.eq(after, before)
  end,

  test_thinking_same_target_autonomous_and_receiver_activation_edges_coexist = function()
    local owner = core.restart_package_name
    local rows = core.restart_transition_table()
    local autonomous_by_id = {}
    for _, edge in ipairs(restart_edges.extract_autonomous_edges(owner, rows)) do
      autonomous_by_id[edge.id] = edge
    end
    local entry_by_id = {}
    for _, edge in ipairs(restart_edges.extract_entry_edges(owner, entry_inventory, rows)) do
      entry_by_id[edge.id] = edge
    end

    local stalled = autonomous_by_id[owner .. "/thinking/autonomous/consensus-stalled"]
    local reconcile = entry_by_id[owner .. "/thinking/entry/issue_reconcile_true_stall"]
    t.is_true(stalled ~= nil)
    t.is_true(reconcile ~= nil)
    t.eq(stalled.target, "blocked")
    t.eq(reconcile.target, "blocked")
  end,

  test_restart_edges_do_not_read_to_states_or_override_explicit_kinds = function()
    local synthetic = setmetatable({
      from_state = "authored-order",
      responsibility_signature = {
        successors = {
          { state = "z-target", output_variant = "z-first", kind = "timeout" },
          { state = "a-target", output_variant = "a-second", kind = "guard_boundary" },
        },
      },
    }, {
      __index = function(_, key)
        if key == "to_states" then
          error("to_states must not be read")
        end
        return nil
      end,
    })
    local edges = restart_edges.extract_autonomous_edges("issue-owner", { synthetic })
    t.eq(#edges, 0)
    local timeout_edges = restart_edges.extract_timeout_edges("issue-owner", {
      setmetatable({
        from_state = synthetic.from_state,
        actionable_epoch = { source = "state_entry:v1" },
        responsibility_signature = {
          successors = { synthetic.responsibility_signature.successors[1] },
        },
      }, getmetatable(synthetic)),
    })
    t.eq(#timeout_edges, 1)
    t.eq(timeout_edges[1].id, "issue-owner/authored-order/timeout/z-first")
    local guard_edges = restart_edges.extract_guard_boundary_edges("issue-owner", { synthetic })
    t.eq(#guard_edges, 1)
    t.eq(guard_edges[1].id, "issue-owner/authored-order/guard_boundary/a-second")
  end,

  test_restart_guard_boundary_edges_do_not_read_to_states_sort_or_leak_autonomous = function()
    local synthetic = setmetatable({
      from_state = "authored-order",
      responsibility_signature = {
        successors = {
          { state = "autonomous-target", output_variant = "autonomous-only", kind = "autonomous" },
        },
      },
      guard_boundaries = {
        {
          name = "z-boundary",
          successors = {
            { state = "z-target", output_variant = "z-first" },
            { state = "a-target", output_variant = "a-second" },
          },
        },
        {
          name = "a-boundary",
          successors = {
            { state = "m-target", output_variant = "m-third" },
          },
        },
      },
    }, {
      __index = function(_, key)
        if key == "to_states" then
          error("to_states must not be read")
        end
        return nil
      end,
    })
    local expected, rows_without_boundaries = expected_guard_boundary_edges("issue-owner", { synthetic })
    local edges = restart_edges.extract_guard_boundary_edges("issue-owner", { synthetic })
    assert_guard_boundary_edges(edges, expected, rows_without_boundaries)
    t.eq(#edges, 3)
    t.eq(edges[1].id, "issue-owner/authored-order/guard_boundary/z-boundary/z-first")
    t.eq(edges[1].source.boundary, "z-boundary")
    t.eq(edges[2].id, "issue-owner/authored-order/guard_boundary/z-boundary/a-second")
    t.eq(edges[2].source.boundary, "z-boundary")
    t.eq(edges[3].id, "issue-owner/authored-order/guard_boundary/a-boundary/m-third")
    t.eq(edges[3].source.boundary, "a-boundary")

    local repeated = restart_edges.extract_guard_boundary_edges("issue-owner", { synthetic })
    assert_guard_boundary_edges(repeated, expected, rows_without_boundaries)
    for index, edge in ipairs(edges) do
      t.is_true(edge ~= repeated[index])
      t.is_true(edge.source ~= repeated[index].source)
      t.is_true(edge.provenance ~= repeated[index].provenance)
    end
  end,

  test_restart_edges_exclude_blocked_operator_reentry = function()
    local owner = core.restart_package_name
    local rows = core.restart_transition_table()
    local blocked = row_by_state(rows, "blocked")
    t.eq(blocked.operator_reentry.not_autonomous_successor, true)
    t.eq(blocked.responsibility_signature.operator_reentry.not_autonomous_successor, true)
    t.eq(#blocked.responsibility_signature.successors, 0)
    for _, edge in ipairs(restart_edges.extract_autonomous_edges(owner, rows)) do
      t.is_true(edge.row_id ~= "blocked")
    end
  end,

  test_restart_edges_fail_closed_on_invalid_authored_inputs = function()
    local valid = row("from", { { state = "to", output_variant = "done", kind = "autonomous" } })
    assert_extract_fails("", { valid })
    assert_extract_fails("owner", { row(nil, { { state = "to", output_variant = "done", kind = "autonomous" } }) })
    assert_extract_fails("owner", { row("", { { state = "to", output_variant = "done", kind = "autonomous" } }) })
    assert_extract_fails("owner", { { from_state = "from", responsibility_signature = {} } })
    assert_extract_fails("owner", { row("from", { { output_variant = "done", kind = "autonomous" } }) })
    assert_extract_fails("owner", { row("from", { { state = "", output_variant = "done", kind = "autonomous" } }) })
    assert_extract_fails("owner", { row("from", { { state = "to", kind = "autonomous" } }) })
    assert_extract_fails("owner", { row("from", { { state = "to", output_variant = "", kind = "autonomous" } }) })
    assert_extract_fails("owner", { row("from", { { state = "to", output_variant = "done" } }) })
    assert_extract_fails("owner", { row("from", { { state = "to", output_variant = "done", kind = "other" } }) })
    assert_extract_fails("owner", {
      row("from", {
        { state = "one", output_variant = "same", kind = "autonomous" },
        { state = "two", output_variant = "same", kind = "autonomous" },
      }),
    })

    assert_timeout_extract_fails("", { valid })
    assert_timeout_extract_fails("owner", {
      row("from", { { state = "to", output_variant = "done", kind = "timeout" } }),
    })
    assert_timeout_extract_fails("owner", {
      {
        from_state = "from",
        actionable_epoch = { source = "unknown:v1" },
        responsibility_signature = {
          successors = { { state = "to", output_variant = "done", kind = "timeout" } },
        },
      },
    })

    local valid_guard = guard_row("from", {
      {
        name = "boundary",
        successors = { { state = "to", output_variant = "done" } },
      },
    })
    assert_guard_extract_fails("", { valid_guard })
    assert_guard_extract_fails("owner", { guard_row(nil, {}) })
    assert_guard_extract_fails("owner", { guard_row("", {}) })
    assert_guard_extract_fails("owner", { guard_row("from", { { successors = {} } }) })
    assert_guard_extract_fails("owner", { guard_row("from", { { name = "", successors = {} } }) })
    assert_guard_extract_fails("owner", { guard_row("from", { { name = "boundary" } }) })
    assert_guard_extract_fails("owner", {
      guard_row("from", { { name = "boundary", successors = { { output_variant = "done" } } } }),
    })
    assert_guard_extract_fails("owner", {
      guard_row("from", { { name = "boundary", successors = { { state = "", output_variant = "done" } } } }),
    })
    assert_guard_extract_fails("owner", {
      guard_row("from", { { name = "boundary", successors = { { state = "to" } } } }),
    })
    assert_guard_extract_fails("owner", {
      guard_row("from", { { name = "boundary", successors = { { state = "to", output_variant = "" } } } }),
    })
    assert_guard_extract_fails("owner", {
      guard_row("from", {
        {
          name = "boundary",
          successors = {
            { state = "one", output_variant = "same" },
            { state = "two", output_variant = "same" },
          },
        },
      }),
    })
  end,
}
