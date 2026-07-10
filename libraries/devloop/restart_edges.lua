local M = {}

local function is_nonempty_string(value)
  return type(value) == "string" and value ~= ""
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

function M.schema()
  return {
    structural_fields = { "id", "owner", "row_id", "kind", "source", "target", "provenance" },
    extracted_kinds = { autonomous = true },
    deferred_kinds = { "entry", "operator_reentry", "timeout", "guard_boundary", "canonicalization" },
  }
end

return M
