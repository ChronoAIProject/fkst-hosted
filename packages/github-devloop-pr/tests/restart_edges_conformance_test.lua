local h = require("tests.devloop_core_helpers")
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

local deferred_kinds = {
  "timeout",
  "canonicalization",
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

local function expected_edges(owner, rows)
  local expected = {}
  local empty_rows = {}
  for _, row in ipairs(rows) do
    local successors = row.responsibility_signature.successors
    if #successors == 0 then
      table.insert(empty_rows, row.from_state)
    end
    for _, successor in ipairs(successors) do
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
  return expected, empty_rows
end

local function expected_guard_boundary_edges(owner, rows)
  local expected = {}
  local rows_without_boundaries = {}
  for _, row in ipairs(rows) do
    if row.guard_boundaries == nil then
      table.insert(rows_without_boundaries, row.from_state)
    else
      for _, guard_boundary in ipairs(row.guard_boundaries) do
        for _, successor in ipairs(guard_boundary.successors) do
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
  return expected, rows_without_boundaries
end

local function assert_edges(actual, expected, empty_rows)
  t.eq(#actual, #expected)
  local edge_keys = key_set(structural_fields)
  local seen_ids = {}
  local seen_edges = {}
  local seen_sources = {}
  local seen_provenance = {}
  local counts_by_row = {}
  for index, expected_edge in ipairs(expected) do
    local edge = actual[index]
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
  local edge_keys = key_set(structural_fields)
  local seen_ids = {}
  local seen_edges = {}
  local seen_sources = {}
  local seen_provenance = {}
  local counts_by_row = {}
  for index, expected_edge in ipairs(expected) do
    local edge = actual[index]
    assert_exact_keys(edge, edge_keys)
    assert_exact_keys(edge.source, { state = true, boundary = true })
    assert_exact_keys(edge.provenance, { owner = true, row = true, field = true })
    t.eq(edge.id, expected_edge.id)
    t.eq(edge.owner, expected_edge.owner)
    t.eq(edge.row_id, expected_edge.row_id)
    t.eq(edge.kind, expected_edge.kind)
    t.eq(edge.source.state, expected_edge.source.state)
    t.eq(edge.source.boundary, expected_edge.source.boundary)
    t.eq(edge.target, expected_edge.target)
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
      extract_entry_edges = true,
      extract_guard_boundary_edges = true,
      extract_operator_reentry_edges = true,
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
      entry = true,
      guard_boundary = true,
      operator_reentry = true,
    })
    t.eq(schema.extracted_kinds.autonomous, true)
    t.eq(schema.extracted_kinds.entry, true)
    t.eq(schema.extracted_kinds.guard_boundary, true)
    t.eq(schema.extracted_kinds.operator_reentry, true)
    t.eq(#schema.deferred_kinds, #deferred_kinds)
    for index, kind in ipairs(deferred_kinds) do
      t.eq(schema.deferred_kinds[index], kind)
    end
  end,

  test_restart_edges_match_pr_rows_in_registry_and_authored_successor_order = function()
    local owner = core.restart_package_name
    local rows = core.restart_transition_table()
    local snapshot = copy_value(rows)
    local expected, empty_rows = expected_edges(owner, rows)
    local actual = restart_edges.extract_autonomous_edges(owner, rows)
    assert_edges(actual, expected, empty_rows)
    assert_same_value(rows, snapshot)

    local repeated = restart_edges.extract_autonomous_edges(owner, rows)
    assert_edges(repeated, expected, empty_rows)
    for index, edge in ipairs(actual) do
      t.is_true(edge ~= repeated[index])
      t.is_true(edge.source ~= repeated[index].source)
      t.is_true(edge.provenance ~= repeated[index].provenance)
    end

    -- Timeout and canonicalization edges remain deferred.
    -- This conformance does not claim OLD/live-transition parity: thinking->dependency_wait
    -- and pr-open->blocked exist in production but not in these rows, so they are out of scope.
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
      { state = "autonomous-target", output_variant = "autonomous-output" },
    })
    t.eq(#restart_edges.extract_guard_boundary_edges(owner, { row_without_boundaries }), 0)
  end,

  test_restart_edges_do_not_read_to_states_or_infer_a_different_kind = function()
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
    t.eq(#edges, 2)
    t.eq(edges[1].id, "pr-owner/authored-order/autonomous/z-first")
    t.eq(edges[1].kind, "autonomous")
    t.eq(edges[2].id, "pr-owner/authored-order/autonomous/a-second")
    t.eq(edges[2].kind, "autonomous")
  end,

  test_restart_guard_boundary_edges_do_not_read_to_states_sort_or_leak_autonomous = function()
    local synthetic = setmetatable({
      from_state = "authored-order",
      responsibility_signature = {
        successors = {
          { state = "autonomous-target", output_variant = "autonomous-only" },
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

  test_restart_edges_exclude_merge_ready_guard_boundary_successors = function()
    local owner = core.restart_package_name
    local rows = core.restart_transition_table()
    local merge_ready = row_by_state(rows, "merge-ready")
    local guard = merge_ready.guard_boundaries[1]
    t.eq(guard.kind, "guard_table")
    t.is_true(#guard.successors > 0)
    local ids = {}
    for _, edge in ipairs(restart_edges.extract_autonomous_edges(owner, rows)) do
      ids[edge.id] = true
    end
    for _, successor in ipairs(guard.successors) do
      local guard_id = owner .. "/merge-ready/autonomous/" .. successor.output_variant
      t.eq(ids[guard_id], nil)
    end
  end,

  test_restart_edges_fail_closed_on_invalid_authored_inputs = function()
    local valid = row("from", { { state = "to", output_variant = "done" } })
    assert_extract_fails("", { valid })
    assert_extract_fails("owner", { row(nil, { { state = "to", output_variant = "done" } }) })
    assert_extract_fails("owner", { row("", { { state = "to", output_variant = "done" } }) })
    assert_extract_fails("owner", { { from_state = "from", responsibility_signature = {} } })
    assert_extract_fails("owner", { row("from", { { output_variant = "done" } }) })
    assert_extract_fails("owner", { row("from", { { state = "", output_variant = "done" } }) })
    assert_extract_fails("owner", { row("from", { { state = "to" } }) })
    assert_extract_fails("owner", { row("from", { { state = "to", output_variant = "" } }) })
    assert_extract_fails("owner", {
      row("from", {
        { state = "one", output_variant = "same" },
        { state = "two", output_variant = "same" },
      }),
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
