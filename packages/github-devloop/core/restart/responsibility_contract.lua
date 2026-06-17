local S = {}

function S.install(M)

local state_kinds = {
  queue_wait = true,
  worker = true,
  decision = true,
  gate = true,
  terminal_hold = true,
}

local known_god_states = {
  ready = {
    ["ready: non-terminal row must declare responsibility_signature"] = "dependency gate and implementation kickoff are still fused until the ready split.",
  },
  reviewing = {
    ["reviewing: non-terminal row must declare responsibility_signature"] = "Review decision, convergence heartbeat, failure, and meta fallback are still fused until the reviewing split.",
  },
  ["merge-ready"] = {
    ["merge-ready: non-terminal row must declare responsibility_signature"] = "Worst god-state: CI wait, merge start, review carry-over/backward review, fix fallback, and block fallback are still fused.",
  },
  implementing = {
    ["implementing: non-terminal row must declare responsibility_signature"] = "Implementation success and implementation terminal failure are still fused until the implementing split.",
  },
  fixing = {
    ["fixing: non-terminal row must declare responsibility_signature"] = "Fix execution and review-meta fallback are still fused until the fixing split.",
  },
  blocked = {
    ["blocked: non-terminal row must declare responsibility_signature"] = "Terminal hold, operator reentry, and decomposition fanout are still fused until the blocked split.",
  },
}

local function copy_table(map)
  local out = {}
  for key, value in pairs(map or {}) do
    if type(value) == "table" then
      local nested = {}
      for nested_key, nested_value in pairs(value) do
        nested[nested_key] = nested_value
      end
      out[key] = nested
    else
      out[key] = value
    end
  end
  return out
end

function M.known_god_states()
  return copy_table(known_god_states)
end

local function state_name(row)
  return tostring(row and (row.from_state or row.state) or "?")
end

local function non_empty_string(value)
  return type(value) == "string" and value ~= ""
end

local function list_count(values)
  if type(values) ~= "table" then
    return 0
  end
  local count = 0
  for _, _ in ipairs(values) do
    count = count + 1
  end
  return count
end

local function has_value(values, expected)
  for _, value in ipairs(values or {}) do
    if value == expected then
      return true
    end
  end
  return false
end

local function edge_by_state(signature)
  local by_state = {}
  for _, edge in ipairs(signature and signature.successors or {}) do
    if non_empty_string(edge.state) then
      by_state[edge.state] = edge
    end
  end
  return by_state
end

local function copy_edge(edge, state)
  local out = {}
  if type(edge) == "table" then
    for key, value in pairs(edge) do
      out[key] = value
    end
  end
  out.state = state
  return out
end

local function actual_successor_edges(row, signature)
  local by_state = edge_by_state(signature)
  local out = {}
  for _, next_state in ipairs(row.to_states or {}) do
    table.insert(out, copy_edge(by_state[next_state], next_state))
  end
  return out
end

local function edge_is_terminal(edge)
  return edge ~= nil and edge.terminal == true
end

local function edge_is_failure(edge)
  return edge ~= nil and edge.failure == true
end

local function edge_is_normal(edge)
  return not edge_is_terminal(edge) and not edge_is_failure(edge)
end

local function successor_list(subject)
  if type(subject) ~= "table" then
    return {}
  end
  return subject.successors or subject
end

local function normal_edges(subject)
  local out = {}
  for _, edge in ipairs(successor_list(subject)) do
    if edge_is_normal(edge) then
      table.insert(out, edge)
    end
  end
  return out
end

local function failure_edges(subject)
  local out = {}
  for _, edge in ipairs(successor_list(subject)) do
    if edge_is_failure(edge) then
      table.insert(out, edge)
    end
  end
  return out
end

local function missing_signature_field(signature, field)
  return signature[field] == nil
    or (type(signature[field]) == "string" and signature[field] == "")
    or (type(signature[field]) == "table" and next(signature[field]) == nil)
end

local function validate_signature_shape(row, signature, errors)
  local state = state_name(row)
  if type(signature.receiver_kind) == "table" then
    table.insert(errors, state .. ": responsibility_signature.receiver_kind must be exactly one receiver")
  elseif not non_empty_string(signature.receiver_kind) then
    table.insert(errors, state .. ": responsibility_signature.receiver_kind must be a non-empty string")
  end
  if signature.driving_queue ~= row.driving_queue then
    table.insert(errors, state .. ": responsibility_signature.driving_queue must match row.driving_queue")
  end
  if state_kinds[signature.state_kind] ~= true then
    table.insert(errors, state .. ": responsibility_signature.state_kind must be queue_wait, worker, decision, gate, or terminal_hold")
  end
  if signature.liveness_class ~= row.liveness_class_id then
    table.insert(errors, state .. ": responsibility_signature.liveness_class must match row.liveness_class_id")
  end
  for _, field in ipairs({ "input_fact_family", "output_postcondition_family" }) do
    if missing_signature_field(signature, field) then
      table.insert(errors, state .. ": responsibility_signature." .. field .. " must be declared")
    end
  end
  if tonumber(signature.phase_rank) == nil then
    table.insert(errors, state .. ": responsibility_signature.phase_rank must be declared")
  elseif M.stage_rank ~= nil and tonumber(signature.phase_rank) ~= M.stage_rank(row.from_state) then
    table.insert(errors, state .. ": responsibility_signature.phase_rank must match stage_rank")
  end
  if type(signature.lineage_keys) ~= "table" or #signature.lineage_keys == 0 then
    table.insert(errors, state .. ": responsibility_signature.lineage_keys must be declared")
  end
  if type(signature.successors) ~= "table" then
    table.insert(errors, state .. ": responsibility_signature.successors must be declared")
  end
end

local function validate_successor_coverage(row, signature, errors)
  local state = state_name(row)
  local by_state = edge_by_state(signature)
  local declared_seen = {}
  for _, next_state in ipairs(row.to_states or {}) do
    if by_state[next_state] == nil then
      table.insert(errors, state .. ": responsibility_signature.successors missing row successor " .. tostring(next_state))
    end
  end
  for _, edge in ipairs(signature.successors or {}) do
    if not non_empty_string(edge.state) then
      table.insert(errors, state .. ": responsibility_signature.successor.state must be declared")
    elseif declared_seen[edge.state] == true then
      table.insert(errors, state .. ": responsibility_signature.successors duplicate state: " .. tostring(edge.state))
    elseif not has_value(row.to_states or {}, edge.state) then
      table.insert(errors, state .. ": responsibility_signature.successor is not in row.to_states: " .. tostring(edge.state))
    end
    if non_empty_string(edge.state) then
      declared_seen[edge.state] = true
    end
    if not non_empty_string(edge.output_variant) then
      table.insert(errors, state .. ": responsibility_signature.successor output_variant must be declared")
    end
    if edge.monotonic ~= true and edge.bump ~= true then
      table.insert(errors, state .. ": responsibility_signature.successor must declare monotonic or bump")
    end
  end
  return actual_successor_edges(row, signature)
end

local function validate_output_family(row, signature, edges, errors)
  local state = state_name(row)
  for _, edge in ipairs(normal_edges(edges)) do
    if edge.postcondition_family ~= nil
      and edge.postcondition_family ~= signature.output_postcondition_family then
      table.insert(errors, state .. ": normal successor has unrelated output_postcondition_family: " .. tostring(edge.state))
    end
  end
end

local function validate_kind_fanout(row, signature, edges, errors)
  local state = state_name(row)
  local normal = normal_edges(edges)
  local failures = failure_edges(edges)
  if signature.state_kind == "queue_wait" then
    if #normal ~= 1 then
      table.insert(errors, state .. ": queue_wait state must declare exactly one normal successor")
    end
    for _, edge in ipairs(edges or {}) do
      if not edge_is_normal(edge) and edge_is_terminal(edge) ~= true then
        table.insert(errors, state .. ": queue_wait may only add terminal cancel/block successors")
      end
    end
  elseif signature.state_kind == "worker" then
    if #normal ~= 1 then
      table.insert(errors, state .. ": worker state must declare exactly one success successor family")
    end
    if #failures > 1 then
      table.insert(errors, state .. ": worker state may declare at most one failure successor family")
    end
  elseif signature.state_kind == "decision" or signature.state_kind == "gate" then
    if #normal == 0 then
      table.insert(errors, state .. ": " .. tostring(signature.state_kind) .. " state must declare a decision successor")
    end
    if not non_empty_string(signature.decision_type) then
      table.insert(errors, state .. ": " .. tostring(signature.state_kind) .. " state must declare decision_type")
    end
    for _, edge in ipairs(normal) do
      if edge.decision_type ~= signature.decision_type then
        table.insert(errors, state .. ": decision successor must be a variant of decision_type " .. tostring(signature.decision_type))
      end
    end
  elseif signature.state_kind == "terminal_hold" then
    if list_count(signature.successors) > 0 or list_count(row.to_states) > 0 then
      table.insert(errors, state .. ": terminal_hold state must not declare autonomous successors")
    end
  end
end

local function validate_phase_monotonicity(row, signature, edges, errors)
  local state = state_name(row)
  local current_rank = tonumber(signature.phase_rank)
  for _, edge in ipairs(edges or {}) do
    local next_rank = M.stage_rank and M.stage_rank(edge.state) or nil
    if current_rank ~= nil and next_rank ~= nil and next_rank < current_rank and edge.bump ~= true then
      table.insert(errors, state .. ": backward successor requires generation bump: " .. tostring(edge.state))
    end
  end
end

local function canonical_value(value)
  if type(value) ~= "table" then
    return tostring(value)
  end
  local keys = {}
  for key, _ in pairs(value) do
    table.insert(keys, key)
  end
  table.sort(keys, function(a, b) return tostring(a) < tostring(b) end)
  local parts = {}
  for _, key in ipairs(keys) do
    table.insert(parts, tostring(key) .. "=" .. canonical_value(value[key]))
  end
  return "{" .. table.concat(parts, ",") .. "}"
end

local function responsibility_fingerprint(signature)
  return table.concat({
    canonical_value(signature.receiver_kind),
    canonical_value(signature.driving_queue),
    canonical_value(signature.state_kind),
    canonical_value(signature.liveness_class),
    canonical_value(signature.input_fact_family),
    canonical_value(signature.output_postcondition_family),
    canonical_value(signature.phase_rank),
    canonical_value(signature.lineage_keys),
  }, "|")
end

local function validate_unique_signature(row, signature, seen, errors)
  local fingerprint = responsibility_fingerprint(signature)
  local previous = seen[fingerprint]
  if previous ~= nil then
    table.insert(errors, state_name(row) .. ": duplicate responsibility_signature shared with " .. tostring(previous))
    return
  end
  seen[fingerprint] = row.from_state
end

local function validate_row(row, seen, errors)
  if row == nil or row.terminal == true then
    return
  end
  local state = state_name(row)
  local signature = row.responsibility_signature
  if type(signature) ~= "table" then
    table.insert(errors, state .. ": non-terminal row must declare responsibility_signature")
    return
  end
  validate_signature_shape(row, signature, errors)
  local actual_edges = validate_successor_coverage(row, signature, errors)
  validate_output_family(row, signature, actual_edges, errors)
  validate_kind_fanout(row, signature, actual_edges, errors)
  validate_phase_monotonicity(row, signature, actual_edges, errors)
  validate_unique_signature(row, signature, seen, errors)
end

function M.strict_restart_responsibility_contract_errors(rows)
  local errors = {}
  local seen = {}
  for _, row in ipairs(rows or M.restart_transition_table()) do
    validate_row(row, seen, errors)
  end
  return errors
end

local function error_state(error_text)
  return tostring(error_text or ""):match("^([^:]+):")
end

function M.restart_responsibility_inventory_errors(rows, inventory)
  local strict_errors = M.strict_restart_responsibility_contract_errors(rows)
  local listed = inventory or known_god_states
  local observed_listed_errors = {}
  local errors = {}
  for _, err in ipairs(strict_errors) do
    local state = error_state(err)
    local expected = state ~= nil and listed[state] or nil
    if type(expected) == "table" and expected[err] ~= nil then
      observed_listed_errors[err] = true
      goto continue
    end
    table.insert(errors, err)
    ::continue::
  end
  for state, expected_errors in pairs(listed) do
    for expected_error, _ in pairs(expected_errors or {}) do
      if observed_listed_errors[expected_error] ~= true then
        table.insert(errors, state .. ": listed known_god_states entry is stale and must be removed: " .. expected_error)
      end
    end
  end
  return errors
end

function M.responsibility_contract_inventory_is_listed_violation(state, errors)
  for _, err in ipairs(errors or {}) do
    if error_state(err) == state then
      return true
    end
  end
  return false
end

end

return S
