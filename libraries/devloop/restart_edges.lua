local M = {}

local function is_nonempty_string(value)
  return type(value) == "string" and value ~= ""
end

function M.extract_entry_edges(owner, inventory)
  if not is_nonempty_string(owner) then
    error("devloop.restart_edges: owner must be a non-empty string")
  end
  if type(inventory) ~= "table" then
    error("devloop.restart_edges: entry inventory must be a table")
  end

  local edges = {}
  local seen_ids = {}
  for _, authored in ipairs(inventory) do
    if type(authored) ~= "table" then
      error("devloop.restart_edges: entry edge must be a table")
    end
    if not is_nonempty_string(authored.id) then
      error("devloop.restart_edges: entry edge id must be a non-empty string")
    end
    if authored.owner ~= owner then
      error("devloop.restart_edges: entry edge owner must match extractor owner")
    end
    if not is_nonempty_string(authored.row_id) then
      error("devloop.restart_edges: entry edge row_id must be a non-empty string")
    end
    if authored.kind ~= "entry" then
      error("devloop.restart_edges: entry edge kind must be entry")
    end

    local source = authored.source
    if type(source) ~= "table" then
      error("devloop.restart_edges: entry edge source must be a table")
    end
    if source.state ~= nil then
      error("devloop.restart_edges: entry edge source.state must be nil")
    end
    if not is_nonempty_string(source.boundary) then
      error("devloop.restart_edges: entry edge source.boundary must be a non-empty string")
    end
    if not is_nonempty_string(authored.target) then
      error("devloop.restart_edges: entry edge target must be a non-empty string")
    end

    local provenance = authored.provenance
    if type(provenance) ~= "table" then
      error("devloop.restart_edges: entry edge provenance must be a table")
    end
    if provenance.owner ~= owner then
      error("devloop.restart_edges: entry edge provenance.owner must match extractor owner")
    end
    if not is_nonempty_string(provenance.row) then
      error("devloop.restart_edges: entry edge provenance.row must be a non-empty string")
    end
    if not is_nonempty_string(provenance.field) then
      error("devloop.restart_edges: entry edge provenance.field must be a non-empty string")
    end
    if seen_ids[authored.id] then
      error("devloop.restart_edges: duplicate edge id " .. authored.id)
    end
    seen_ids[authored.id] = true

    table.insert(edges, {
      id = authored.id,
      owner = authored.owner,
      row_id = authored.row_id,
      kind = authored.kind,
      source = {
        state = nil,
        boundary = source.boundary,
      },
      target = authored.target,
      provenance = {
        owner = provenance.owner,
        row = provenance.row,
        field = provenance.field,
      },
    })
  end
  return edges
end

function M.extract_autonomous_edges(owner, rows)
  if not is_nonempty_string(owner) then
    error("devloop.restart_edges: owner must be a non-empty string")
  end

  local edges = {}
  local seen_ids = {}
  for _, row in ipairs(rows) do
    local row_id = row.from_state
    if not is_nonempty_string(row_id) then
      error("devloop.restart_edges: row.from_state must be a non-empty string")
    end

    local signature = row.responsibility_signature
    local successors = type(signature) == "table" and signature.successors or nil
    if type(successors) ~= "table" then
      error("devloop.restart_edges: responsibility_signature.successors must be a table")
    end

    for _, successor in ipairs(successors) do
      if type(successor) ~= "table" or not is_nonempty_string(successor.state) then
        error("devloop.restart_edges: successor.state must be a non-empty string")
      end
      if not is_nonempty_string(successor.output_variant) then
        error("devloop.restart_edges: successor.output_variant must be a non-empty string")
      end

      local id = owner .. "/" .. row_id .. "/autonomous/" .. successor.output_variant
      if seen_ids[id] then
        error("devloop.restart_edges: duplicate edge id " .. id)
      end
      seen_ids[id] = true
      table.insert(edges, {
        id = id,
        owner = owner,
        row_id = row_id,
        kind = "autonomous",
        source = { state = row_id, boundary = nil },
        target = successor.state,
        provenance = {
          owner = owner,
          row = row_id,
          field = "responsibility_signature.successors",
        },
      })
    end
  end
  return edges
end

function M.extract_guard_boundary_edges(owner, rows)
  if not is_nonempty_string(owner) then
    error("devloop.restart_edges: owner must be a non-empty string")
  end

  local edges = {}
  local seen_ids = {}
  for _, row in ipairs(rows) do
    local row_id = row.from_state
    if not is_nonempty_string(row_id) then
      error("devloop.restart_edges: row.from_state must be a non-empty string")
    end

    local guard_boundaries = row.guard_boundaries
    if guard_boundaries ~= nil then
      if type(guard_boundaries) ~= "table" then
        error("devloop.restart_edges: guard_boundaries must be a table")
      end

      for _, guard_boundary in ipairs(guard_boundaries) do
        if type(guard_boundary) ~= "table" or not is_nonempty_string(guard_boundary.name) then
          error("devloop.restart_edges: guard_boundary.name must be a non-empty string")
        end

        local successors = guard_boundary.successors
        if type(successors) ~= "table" then
          error("devloop.restart_edges: guard_boundary.successors must be a table")
        end

        for _, successor in ipairs(successors) do
          if type(successor) ~= "table" or not is_nonempty_string(successor.state) then
            error("devloop.restart_edges: successor.state must be a non-empty string")
          end
          if not is_nonempty_string(successor.output_variant) then
            error("devloop.restart_edges: successor.output_variant must be a non-empty string")
          end

          local id = owner .. "/" .. row_id .. "/guard_boundary/" .. guard_boundary.name .. "/" .. successor.output_variant
          if seen_ids[id] then
            error("devloop.restart_edges: duplicate edge id " .. id)
          end
          seen_ids[id] = true
          table.insert(edges, {
            id = id,
            owner = owner,
            row_id = row_id,
            kind = "guard_boundary",
            source = { state = row_id, boundary = guard_boundary.name },
            target = successor.state,
            provenance = {
              owner = owner,
              row = row_id,
              field = "guard_boundaries",
            },
          })
        end
      end
    end
  end
  return edges
end

function M.schema()
  return {
    structural_fields = { "id", "owner", "row_id", "kind", "source", "target", "provenance" },
    -- Entry extraction is inventory-driven; the PR-owner inventory is a later increment.
    extracted_kinds = { autonomous = true, entry = true, guard_boundary = true },
    deferred_kinds = { "operator_reentry", "timeout", "canonicalization" },
  }
end

return M
