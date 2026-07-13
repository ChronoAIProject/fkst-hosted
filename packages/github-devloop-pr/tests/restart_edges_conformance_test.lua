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
  ["fixing/revision_published"] = "autonomous",
  ["fixing/revision_failed"] = "autonomous",
  ["fixing/fix_budget_exhausted"] = "autonomous",
  ["merge-ready/fix_budget_exhausted"] = "autonomous",
  ["merging/merge-completed"] = "autonomous",
  ["merging/head-advanced"] = "autonomous",
  ["merging/merge-needs-fix"] = "autonomous",
  ["merging/fix_budget_exhausted"] = "autonomous",
  ["pr-open/review_requested"] = "autonomous",
  ["pr-open/not_mergeable_repair"] = "autonomous",
  ["pr-open/pr_base_unmanaged"] = "guard_boundary",
  ["review-meta/fix"] = "autonomous",
  ["review-meta/block"] = "autonomous",
  ["reviewing/approved"] = "autonomous",
  ["reviewing/changes_requested"] = "autonomous",
  ["reviewing/needs_review_meta"] = "autonomous",
  ["reviewing/watchdog_reconcile_terminal"] = "timeout",
}

local expected_real_cas_by_id = {
  ["github-devloop-pr/reviewing/timeout/watchdog_reconcile_terminal"] = {
    cas_policy_id = "cas.legacy_timeout_reconcile_v1",
    cas_variant = "reviewing_to_blocked",
  },
  ["github-devloop-pr/merge-ready/timeout/merge_gate/watchdog_reconcile_terminal"] = {
    cas_policy_id = "cas.legacy_timeout_reconcile_v1",
    cas_variant = "merge_ready_to_blocked",
  },
  ["github-devloop-pr/reviewing/autonomous/approved"] = {
    cas_policy_id = "cas.legacy_review_result_v1",
    cas_variant = "reviewing_to_merge_ready",
  },
  ["github-devloop-pr/reviewing/autonomous/changes_requested"] = {
    cas_policy_id = "cas.legacy_review_result_v1",
    cas_variant = "reviewing_to_fixing",
  },
  ["github-devloop-pr/reviewing/autonomous/needs_review_meta"] = {
    cas_policy_id = "cas.legacy_review_result_v1",
    cas_variant = "reviewing_to_review_meta",
  },
  ["github-devloop-pr/fixing/autonomous/revision_published"] = {
    cas_policy_id = "cas.legacy_fix_v1",
    cas_variant = "fixing_to_reviewing",
  },
  ["github-devloop-pr/review-meta/autonomous/fix"] = {
    cas_policy_id = "cas.legacy_review_meta_v1",
    cas_variant = "predecision_eligibility",
  },
  ["github-devloop-pr/review-meta/autonomous/block"] = {
    cas_policy_id = "cas.legacy_review_meta_v1",
    cas_variant = "predecision_eligibility",
  },
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

local function assert_semantic_variant(edge)
  if edge.semantic_variant == nil then return end
  t.eq(type(edge.semantic_variant), "string")
  t.is_true(edge.semantic_variant ~= "")
  t.eq(edge.semantic_variant:find("/", 1, true), nil)
  t.eq(edge.semantic_variant, tostring(edge.id):match("/([^/]+)$"))
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

local function assert_valid_cas(edge)
  if edge.cas_policy_id == nil then
    return
  end
  local definition = restart_cas_catalog.definition(edge.cas_policy_id)
  t.is_true(definition ~= nil)
  if edge.cas_variant ~= nil then
    t.is_true(definition.variants ~= nil)
    t.is_true(definition.variants[edge.cas_variant] ~= nil)
  end
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
        local edge_id = owner .. "/" .. row.from_state .. "/autonomous/" .. successor.output_variant
        local expected_cas = expected_real_cas_by_id[edge_id]
        table.insert(expected, {
          id = edge_id,
          owner = owner,
          row_id = row.from_state,
          kind = "autonomous",
          source = { state = row.from_state, boundary = nil },
          target = successor.state,
          semantic_variant = successor.output_variant,
          pending_order = copy_value(successor.pending_order),
          cas_policy_id = expected_cas and expected_cas.cas_policy_id or nil,
          cas_variant = expected_cas and expected_cas.cas_variant or nil,
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
          semantic_variant = successor.output_variant,
          pending_order = copy_value(successor.pending_order),
          provenance = {
            owner = owner,
            row = row.from_state,
            field = "responsibility_signature.successors",
          },
        })
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
              semantic_variant = successor.output_variant,
              pending_order = copy_value(successor.pending_order),
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
        local edge_id = owner .. "/" .. row.from_state .. "/timeout/" .. successor.output_variant
        local expected_cas = expected_real_cas_by_id[edge_id]
        table.insert(expected, {
          id = edge_id,
          owner = owner,
          row_id = row.from_state,
          kind = "timeout",
          source = { state = row.from_state, boundary = nil },
          target = successor.state,
          semantic_variant = successor.output_variant,
          pending_order = copy_value(successor.pending_order),
          timeout_evidence_policy_id = policy_id,
          cas_policy_id = expected_cas and expected_cas.cas_policy_id or nil,
          cas_variant = expected_cas and expected_cas.cas_variant or nil,
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
          local edge_id = owner .. "/" .. row.from_state .. "/timeout/" .. guard_boundary.name .. "/" .. successor.output_variant
          local expected_cas = expected_real_cas_by_id[edge_id]
          table.insert(expected, {
            id = edge_id,
            owner = owner,
            row_id = row.from_state,
            kind = "timeout",
            source = { state = row.from_state, boundary = guard_boundary.name },
            target = successor.state,
            semantic_variant = successor.output_variant,
            pending_order = copy_value(successor.pending_order),
            timeout_evidence_policy_id = policy_id,
            cas_policy_id = expected_cas and expected_cas.cas_policy_id or nil,
            cas_variant = expected_cas and expected_cas.cas_variant or nil,
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
    edge_keys.semantic_variant = true
    if expected_edge.cas_policy_id ~= nil then
      edge_keys.cas_policy_id = true
    end
    if expected_edge.cas_variant ~= nil then
      edge_keys.cas_variant = true
    end
    if expected_edge.pending_order ~= nil then edge_keys.pending_order = true end
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
    t.eq(edge.semantic_variant, expected_edge.semantic_variant)
    assert_semantic_variant(edge)
    t.eq(edge.cas_policy_id, expected_edge.cas_policy_id)
    t.eq(edge.cas_variant, expected_edge.cas_variant)
    assert_same_value(edge.pending_order, expected_edge.pending_order)
    assert_valid_cas(edge)
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
    edge_keys.semantic_variant = true
    if expected_edge.pending_order ~= nil then edge_keys.pending_order = true end
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
    t.eq(edge.semantic_variant, expected_edge.semantic_variant)
    assert_semantic_variant(edge)
    assert_same_value(edge.pending_order, expected_edge.pending_order)
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
  local seen_ids = {}
  for index, expected_edge in ipairs(expected) do
    local edge = actual[index]
    local edge_keys = key_set(structural_fields)
    edge_keys.semantic_variant = true
    edge_keys.timeout_evidence_policy_id = true
    if expected_edge.pending_order ~= nil then edge_keys.pending_order = true end
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
    t.eq(edge.kind, "timeout")
    t.eq(edge.source.state, expected_edge.source.state)
    t.eq(edge.source.boundary, expected_edge.source.boundary)
    t.eq(edge.target, expected_edge.target)
    t.eq(edge.semantic_variant, expected_edge.semantic_variant)
    assert_semantic_variant(edge)
    t.eq(edge.timeout_evidence_policy_id, expected_edge.timeout_evidence_policy_id)
    t.eq(edge.cas_policy_id, expected_edge.cas_policy_id)
    t.eq(edge.cas_variant, expected_edge.cas_variant)
    assert_same_value(edge.pending_order, expected_edge.pending_order)
    assert_valid_cas(edge)
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
      assert_semantic_variant(edge)
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

  test_restart_edges_match_pr_rows_in_registry_and_authored_successor_order = function()
    local owner = core.restart_package_name
    local rows = core.restart_transition_table()
    local snapshot = copy_value(rows)
    assert_successor_kind_partition(rows)
    local expected, empty_rows = expected_edges(owner, rows)
    local actual = restart_edges.extract_autonomous_edges(owner, rows)
    t.eq(#actual, 15)
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

  test_restart_guard_boundary_edges_match_pr_rows_in_authored_order = function()
    local owner = core.restart_package_name
    local rows = core.restart_transition_table()
    local snapshot = copy_value(rows)
    local expected, rows_without_boundaries = expected_guard_boundary_edges(owner, rows)
    local actual = restart_edges.extract_guard_boundary_edges(owner, rows)
    t.eq(#expected, 4)
    assert_guard_boundary_edges(actual, expected, rows_without_boundaries)
    assert_same_value(rows, snapshot)

    local autonomous_ids = {}
    for _, edge in ipairs(restart_edges.extract_autonomous_edges(owner, rows)) do
      autonomous_ids[edge.id] = true
    end
    for _, edge in ipairs(actual) do
      t.eq(autonomous_ids[edge.id], nil)
    end

    local repeated = restart_edges.extract_guard_boundary_edges(owner, rows)
    assert_guard_boundary_edges(repeated, expected, rows_without_boundaries)
    for index, edge in ipairs(actual) do
      t.is_true(edge ~= repeated[index])
      t.is_true(edge.source ~= repeated[index].source)
      t.is_true(edge.provenance ~= repeated[index].provenance)
    end

    local row_without_boundaries = row("autonomous-only", {
      { state = "autonomous-target", output_variant = "autonomous-output", kind = "autonomous" },
    })
    t.eq(#restart_edges.extract_guard_boundary_edges(owner, { row_without_boundaries }), 0)
  end,

  test_restart_timeout_edges_match_pr_rows_and_closed_evidence_policies = function()
    local owner = core.restart_package_name
    local rows = core.restart_transition_table()
    local snapshot = copy_value(rows)
    local expected = expected_timeout_edges(owner, rows)
    local actual = restart_edges.extract_timeout_edges(owner, rows)

    t.eq(#expected, 2)
    assert_timeout_edges(actual, expected)
    t.eq(actual[1].id, owner .. "/merge-ready/timeout/merge_gate/watchdog_reconcile_terminal")
    t.eq(actual[1].timeout_evidence_policy_id, "timeout.state_entry_legacy_v1")
    t.eq(actual[2].id, owner .. "/reviewing/timeout/watchdog_reconcile_terminal")
    t.eq(actual[2].timeout_evidence_policy_id, "timeout.heartbeat_legacy_v1")
    assert_same_value(rows, snapshot)

    local repeated = restart_edges.extract_timeout_edges(owner, rows)
    assert_timeout_edges(repeated, expected)
    for index, edge in ipairs(actual) do
      t.is_true(edge ~= repeated[index])
      t.is_true(edge.source ~= repeated[index].source)
      t.is_true(edge.provenance ~= repeated[index].provenance)
    end
  end,

  test_restart_edge_kind_partition_preserves_the_legacy_pr_union_byte_for_byte = function()
    local owner = core.restart_package_name
    local rows = core.restart_transition_table()
    local entries = restart_edges.extract_entry_edges(owner, entry_inventory, rows)
    local entry_ids = {}
    local entry_counts = {}
    for _, edge in ipairs(entries) do
      entry_ids[edge.id] = true
      entry_counts[edge.row_id] = (entry_counts[edge.row_id] or 0) + 1
    end
    t.eq(#entries, 12)
    t.eq(entry_counts.reviewing, 4)
    t.eq(entry_counts.fixing, 2)
    t.eq(entry_counts["merge-ready"], 3)
    t.eq(entry_counts.merging, 2)
    t.eq(entry_ids[owner .. "/merge-ready/entry/handoff_to_merge_gate"], true)
    for _, state in ipairs({ "fixing", "merge-ready", "merging" }) do
      t.eq(entry_ids[owner .. "/" .. state .. "/entry/review_reject_to_blocked"], true)
      t.eq(entry_ids[owner .. "/" .. state .. "/entry/bounded_fix_to_blocked"], true)
    end
    t.eq(entry_ids[owner .. "/reviewing/entry/review_convergence_round"], true)
    t.eq(entry_ids[owner .. "/reviewing/entry/review_reject_to_blocked"], true)

    local before = legacy_union_bytes(owner, rows, entry_inventory)
    local after = extracted_union_bytes(owner, rows, entry_inventory)
    t.eq(after, before)
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
    local edges = restart_edges.extract_autonomous_edges("pr-owner", { synthetic })
    t.eq(#edges, 0)
    local timeout_edges = restart_edges.extract_timeout_edges("pr-owner", {
      setmetatable({
        from_state = synthetic.from_state,
        actionable_epoch = { source = "state_entry:v1" },
        responsibility_signature = {
          successors = { synthetic.responsibility_signature.successors[1] },
        },
      }, getmetatable(synthetic)),
    })
    t.eq(#timeout_edges, 1)
    t.eq(timeout_edges[1].id, "pr-owner/authored-order/timeout/z-first")
    local guard_edges = restart_edges.extract_guard_boundary_edges("pr-owner", { synthetic })
    t.eq(#guard_edges, 1)
    t.eq(guard_edges[1].id, "pr-owner/authored-order/guard_boundary/a-second")
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
    local expected, rows_without_boundaries = expected_guard_boundary_edges("pr-owner", { synthetic })
    local edges = restart_edges.extract_guard_boundary_edges("pr-owner", { synthetic })
    assert_guard_boundary_edges(edges, expected, rows_without_boundaries)
    t.eq(#edges, 3)
    t.eq(edges[1].id, "pr-owner/authored-order/guard_boundary/z-boundary/z-first")
    t.eq(edges[1].source.boundary, "z-boundary")
    t.eq(edges[2].id, "pr-owner/authored-order/guard_boundary/z-boundary/a-second")
    t.eq(edges[2].source.boundary, "z-boundary")
    t.eq(edges[3].id, "pr-owner/authored-order/guard_boundary/a-boundary/m-third")
    t.eq(edges[3].source.boundary, "a-boundary")
  end,

  test_restart_edges_partition_merge_ready_guard_field_successors = function()
    local owner = core.restart_package_name
    local rows = core.restart_transition_table()
    local merge_ready = row_by_state(rows, "merge-ready")
    local guard = merge_ready.guard_boundaries[1]
    t.eq(guard.kind, "guard_table")
    t.eq(#guard.successors, 4)
    local guard_ids = {}
    for _, edge in ipairs(restart_edges.extract_guard_boundary_edges(owner, rows)) do
      guard_ids[edge.id] = true
    end
    local timeout_ids = {}
    for _, edge in ipairs(restart_edges.extract_timeout_edges(owner, rows)) do
      timeout_ids[edge.id] = true
    end
    for _, successor in ipairs(guard.successors) do
      local guard_id = owner .. "/merge-ready/guard_boundary/merge_gate/" .. successor.output_variant
      local timeout_id = owner .. "/merge-ready/timeout/merge_gate/" .. successor.output_variant
      if successor.output_variant == "watchdog_reconcile_terminal" then
        t.eq(successor.kind, "timeout")
        t.eq(guard_ids[guard_id], nil)
        t.eq(timeout_ids[timeout_id], true)
      else
        t.eq(successor.kind, nil)
        t.eq(guard_ids[guard_id], true)
        t.eq(timeout_ids[timeout_id], nil)
      end
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
