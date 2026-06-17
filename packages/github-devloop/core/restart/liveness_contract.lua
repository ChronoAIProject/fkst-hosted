local S = {}

function S.install(M)

local epoch_sources = {
  ["state_entry:v1"] = {
    durable = true,
    opens_generation = true,
    excludes_deferred_time = false,
    allowed_when = "no_defer_possible",
  },
  ["liveness_substate_entry:v1"] = {
    durable = true,
    opens_generation = true,
    excludes_deferred_time = true,
    allowed_when = "hierarchical_liveness_substate",
  },
  ["defer_clear_fact:v1"] = {
    durable = true,
    opens_generation = true,
    excludes_deferred_time = true,
    requires_clear_fact = true,
  },
  ["live_defer_epoch:v1"] = {
    durable = true,
    opens_generation = true,
    excludes_deferred_time = true,
    requires_live_marker = true,
    requires_clear_fact = true,
    requires_observed_fact = true,
  },
  ["live_defer_heartbeat:v1"] = {
    durable = true,
    opens_generation = "spawn_or_redrive_only",
    excludes_deferred_time = true,
    requires_live_marker = true,
    requires_producer = true,
    requires_freshness_ms = true,
    requires_redrive_opens_generation = true,
    forbids_clear_fact = true,
    forbids_observed_fact = true,
    forbids_clear_opens_generation = true,
  },
}

local known_liveness_contract_violations = {}

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

function M.restart_liveness_epoch_sources()
  return copy_table(epoch_sources)
end

function M.known_liveness_contract_violations()
  return copy_table(known_liveness_contract_violations)
end

local function state_name(row)
  return tostring(row and (row.from_state or row.state) or "?")
end

local function non_empty_string(value)
  return type(value) == "string" and value ~= ""
end

local function watchdog_budget_ms(row)
  return tonumber(row and row.watchdog and row.watchdog.budget_ms)
end

local function validate_watchdog(row, errors)
  local state = state_name(row)
  local watchdog = row and row.watchdog or nil
  if type(watchdog) ~= "table" then
    table.insert(errors, state .. ": non-terminal row must declare watchdog")
    return nil
  end
  if watchdog.mode ~= "row-budget-bounds-receiver" and watchdog.mode ~= "live-defer" then
    table.insert(errors, state .. ": watchdog.mode must be row-budget-bounds-receiver or live-defer")
  end
  local budget_ms = watchdog_budget_ms(row)
  if budget_ms == nil or budget_ms <= 0 then
    table.insert(errors, state .. ": watchdog.budget_ms must be a positive number")
  end
  local budget_minutes = tonumber(row and row.budget and row.budget.minutes)
  if budget_minutes ~= nil and budget_ms ~= nil and budget_ms ~= budget_minutes * 60 * 1000 then
    table.insert(errors, state .. ": watchdog.budget_ms must match budget.minutes")
  end
  return watchdog
end

local function validate_epoch(row, errors)
  local state = state_name(row)
  local epoch = row and row.actionable_epoch or nil
  if type(epoch) ~= "table" or not non_empty_string(epoch.source) then
    local prefix = "non-terminal row"
    if row and row.watchdog and row.watchdog.mode == "live-defer" then
      prefix = "live-defer row"
    end
    table.insert(errors, state .. ": " .. prefix .. " must declare actionable_epoch.source")
    return nil, nil
  end
  local source = epoch_sources[epoch.source]
  if source == nil then
    table.insert(errors, state .. ": actionable_epoch.source is not registered: " .. tostring(epoch.source))
    return epoch, nil
  end
  if source.durable ~= true then
    table.insert(errors, state .. ": actionable_epoch.source must be durable: " .. tostring(epoch.source))
  end
  if source.opens_generation ~= true and source.opens_generation ~= "spawn_or_redrive_only" then
    table.insert(errors, state .. ": actionable_epoch.source must open a generation: " .. tostring(epoch.source))
  end
  if epoch.generation_source ~= "same_as_actionable_epoch" then
    table.insert(errors, state .. ": actionable_epoch.generation_source must be same_as_actionable_epoch")
  end
  return epoch, source
end

local function validate_release_gate_defer(row, source, errors)
  local state = state_name(row)
  local defer = row and row.defer or nil
  if not non_empty_string(defer.live_marker) then
    table.insert(errors, state .. ": release_gate defer must declare live_marker")
  end
  if tonumber(defer.freshness_ms) == nil or tonumber(defer.freshness_ms) <= 0 then
    table.insert(errors, state .. ": release_gate defer must declare freshness_ms")
  end
  if not non_empty_string(defer.clear_fact) then
    table.insert(errors, state .. ": release_gate defer must declare durable clear_fact")
  end
  if not non_empty_string(defer.observed_fact) then
    table.insert(errors, state .. ": release_gate defer must declare durable observed_fact")
  end
  if defer.clear_opens_generation ~= true then
    table.insert(errors, state .. ": release_gate defer.clear_opens_generation must be true")
  end
  if defer.redrive_opens_generation ~= nil then
    table.insert(errors, state .. ": release_gate defer must not declare redrive_opens_generation")
  end
  local epoch_source = row and row.actionable_epoch and row.actionable_epoch.source
  if epoch_source ~= "live_defer_epoch:v1" and epoch_source ~= "defer_clear_fact:v1" then
    table.insert(errors, state .. ": release_gate defer must use live_defer_epoch:v1 or defer_clear_fact:v1")
  end
  if source ~= nil and source.excludes_deferred_time ~= true then
    table.insert(errors, state .. ": live-defer row declares state_entry epoch source which cannot exclude deferred time")
  end
  if source ~= nil and source.allowed_when == "no_defer_possible" then
    table.insert(errors, state .. ": state_entry:v1 is illegal for live-defer rows because deferred time can accrue before actionability")
  end
end

local function registered_heartbeat_producer(row, defer)
  local signal = row and row.liveness_contract and row.liveness_contract.signal
  if type(signal) ~= "table" then
    return false
  end
  if signal.producer ~= defer.producer then
    return false
  end
  if signal.family ~= defer.producer then
    return false
  end
  local binding = type(M.liveness_signal_producer_contract) == "function"
    and M.liveness_signal_producer_contract(defer.producer)
    or nil
  return type(binding) == "table"
end

local function validate_heartbeat_defer(row, errors)
  local state = state_name(row)
  local defer = row and row.defer or nil
  local epoch = row and row.actionable_epoch or nil
  if not non_empty_string(defer.live_marker) then
    table.insert(errors, state .. ": heartbeat defer must declare live_marker")
  end
  if not non_empty_string(defer.producer) then
    table.insert(errors, state .. ": heartbeat defer must declare producer")
  end
  if tonumber(defer.freshness_ms) == nil or tonumber(defer.freshness_ms) <= 0 then
    table.insert(errors, state .. ": heartbeat defer must declare freshness_ms")
  end
  if defer.redrive_opens_generation ~= true then
    table.insert(errors, state .. ": heartbeat defer.redrive_opens_generation must be true")
  end
  if epoch == nil or epoch.source ~= "live_defer_heartbeat:v1" then
    table.insert(errors, state .. ": heartbeat defer must use live_defer_heartbeat:v1")
  end
  if defer.clear_fact ~= nil then
    table.insert(errors, state .. ": heartbeat defer must not declare clear_fact")
  end
  if defer.observed_fact ~= nil then
    table.insert(errors, state .. ": heartbeat defer must not declare observed_fact")
  end
  if defer.clear_opens_generation ~= nil then
    table.insert(errors, state .. ": heartbeat defer must not declare clear_opens_generation")
  end
  local on_stale = row and row.watchdog and row.watchdog.on_stale
  if type(on_stale) ~= "table" or on_stale.op ~= "redrive_receiver" then
    table.insert(errors, state .. ": heartbeat defer must declare watchdog.on_stale.op=redrive_receiver")
  end
  if type(on_stale) == "table" and on_stale.producer ~= nil and on_stale.producer ~= defer.producer then
    table.insert(errors, state .. ": heartbeat defer watchdog.on_stale producer must match defer.producer")
  end
  if not registered_heartbeat_producer(row, defer) then
    table.insert(errors, state .. ": heartbeat defer producer is not a registered live-defer producer: " .. tostring(defer.producer))
  end
end

local function validate_defer(row, source, errors)
  local state = state_name(row)
  local defer = row and row.defer or nil
  if type(defer) ~= "table" then
    table.insert(errors, state .. ": live-defer row must declare defer")
    return
  end
  if defer.kind == "release_gate" then
    validate_release_gate_defer(row, source, errors)
    return
  end
  if defer.kind == "heartbeat" then
    validate_heartbeat_defer(row, errors)
    return
  end
  table.insert(errors, state .. ": live-defer defer.kind must be release_gate or heartbeat")
end

local function validate_row(row, errors)
  if row == nil or row.terminal == true then
    return
  end
  local state = state_name(row)
  if not non_empty_string(row.liveness_class_id) then
    table.insert(errors, state .. ": non-terminal row must declare liveness_class_id")
  end
  local watchdog = validate_watchdog(row, errors)
  local epoch, source = validate_epoch(row, errors)
  local mode = watchdog and watchdog.mode
  if mode == "live-defer" then
    validate_defer(row, source, errors)
  elseif mode == "row-budget-bounds-receiver" then
    if row.defer ~= nil then
      table.insert(errors, state .. ": row-budget-bounds-receiver row must not declare defer")
    end
  end
  if epoch ~= nil and epoch.source == "state_entry:v1" then
    if mode == "live-defer" then
      table.insert(errors, state .. ": state_entry:v1 is illegal for live-defer rows because deferred time can accrue before actionability")
    end
    if row.defer ~= nil then
      table.insert(errors, state .. ": state_entry:v1 rows must not declare defer")
    end
  end
end

local function validate_runtime_provenance(row, errors)
  if row == nil or row.terminal == true or type(row.actionable_epoch) ~= "table" then
    return
  end
  if epoch_sources[row.actionable_epoch.source] == nil then
    return
  end
  if type(M.actionable_epoch_resolve) ~= "function" then
    return
  end
  local state = state_name(row)
  local comments = {}
  local now_seconds = 0
  if row.actionable_epoch.source == "live_defer_epoch:v1" then
    now_seconds = M.iso_timestamp_epoch_seconds("2026-06-03T00:00:01Z")
    comments = {
      {
        author_login = M.trusted_bot_login(),
        created_at = "2026-06-03T00:00:00Z",
        body = M.dependency_release_marker("github-devloop/issue/provenance/repo/1", "restart-liveness-provenance"),
      },
    }
  elseif row.actionable_epoch.source == "live_defer_heartbeat:v1" then
    now_seconds = M.iso_timestamp_epoch_seconds("2026-06-03T00:00:01Z")
  end
  local ok, eval = pcall(M.actionable_epoch_resolve, row, {
    state = row.from_state,
    version = "restart-liveness-provenance",
    proposal_id = "github-devloop/issue/provenance/repo/1",
    marker_created_at = "2026-06-03T00:00:00Z",
  }, {
    proposal_id = "github-devloop/issue/provenance/repo/1",
    current = { comments = comments },
  }, now_seconds)
  if not ok or type(eval) ~= "table" then
    table.insert(errors, state .. ": actionable_epoch resolver failed runtime provenance check")
    return
  end
  if eval.status == "actionable" and eval.epoch_source ~= row.actionable_epoch.source then
    table.insert(errors, state .. ": actionable_epoch runtime provenance must match declared source")
  end
end

function M.normalized_restart_liveness_rows(rows)
  local normalized = {}
  for _, row in ipairs(rows or M.restart_transition_table()) do
    table.insert(normalized, row)
  end
  return normalized
end

function M.strict_restart_liveness_contract_errors(rows)
  local errors = {}
  for _, row in ipairs(M.normalized_restart_liveness_rows(rows)) do
    validate_row(row, errors)
    validate_runtime_provenance(row, errors)
  end
  return errors
end

local function error_state(error_text)
  local state = tostring(error_text or ""):match("^([^:]+):")
  return state
end

function M.restart_liveness_inventory_errors(rows, inventory)
  local strict_errors = M.strict_restart_liveness_contract_errors(rows)
  local listed = inventory or known_liveness_contract_violations
  local observed_listed_errors = {}
  local errors = {}
  for _, err in ipairs(strict_errors) do
    local state = error_state(err)
    local expected = state ~= nil and listed[state] or nil
    if type(expected) == "table" and expected[err] == true then
      observed_listed_errors[err] = true
      goto continue
    end
    table.insert(errors, err)
    ::continue::
  end
  for state, expected_errors in pairs(listed) do
    for expected_error, enabled in pairs(expected_errors or {}) do
      if enabled == true and observed_listed_errors[expected_error] ~= true then
        table.insert(errors, state .. ": listed known_liveness_contract_violations entry is stale and must be removed: " .. expected_error)
      end
    end
  end
  return errors
end

function M.liveness_contract_inventory_is_listed_violation(state, errors)
  for _, err in ipairs(errors or {}) do
    if error_state(err) == state then
      return true
    end
  end
  return false
end

end

return S
