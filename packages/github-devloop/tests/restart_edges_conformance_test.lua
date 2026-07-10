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
  "entry",
  "operator_reentry",
  "timeout",
  "guard_boundary",
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

local function row(from_state, successors)
  return {
    from_state = from_state,
    responsibility_signature = { successors = successors },
  }
end

local function assert_extract_fails(owner, rows)
  local ok = pcall(function()
    restart_edges.extract_autonomous_edges(owner, rows)
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
  test_restart_edges_schema_is_explicitly_autonomous_only = function()
    assert_exact_keys(restart_edges, {
      extract_autonomous_edges = true,
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
    assert_exact_keys(schema.extracted_kinds, { autonomous = true })
    t.eq(schema.extracted_kinds.autonomous, true)
    t.eq(#schema.deferred_kinds, #deferred_kinds)
    for index, kind in ipairs(deferred_kinds) do
      t.eq(schema.deferred_kinds[index], kind)
    end
  end,

  test_restart_edges_match_issue_rows_in_registry_and_authored_successor_order = function()
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

    -- This conformance does not claim OLD/live-transition parity: thinking->dependency_wait
    -- and pr-open->blocked exist in production but not in these rows, so they are out of scope.
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
    local edges = restart_edges.extract_autonomous_edges("issue-owner", { synthetic })
    t.eq(#edges, 2)
    t.eq(edges[1].id, "issue-owner/authored-order/autonomous/z-first")
    t.eq(edges[1].kind, "autonomous")
    t.eq(edges[2].id, "issue-owner/authored-order/autonomous/a-second")
    t.eq(edges[2].kind, "autonomous")
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
  end,
}
