local S = {}

function S.install(M)

local function comment_created_ms(comment)
  local seconds = M.iso_timestamp_epoch_seconds(M._comment_created_at(comment))
  if seconds == nil then
    return nil
  end
  return seconds * 1000
end

local function state_entry_ms(state)
  local seconds = M.iso_timestamp_epoch_seconds(state and state.marker_created_at)
  if seconds == nil then
    return nil
  end
  return seconds * 1000
end

local function marker_attr(marker, name)
  return tostring(marker or ""):match(name .. '="([^"]*)"')
end

local function marker_family(marker_ref)
  local family = tostring(marker_ref or ""):match("^([^:]+):v%d+$")
  return family
end

local function marker_pattern(family)
  local escaped = tostring(family or ""):gsub("%-", "%%-")
  return "<!%-%- fkst:github%-devloop:" .. escaped .. ":v1.-%-%->"
end

local function live_defer_comments(row, facts)
  local signal = row and row.liveness_contract and row.liveness_contract.signal
  if signal and signal.surface == "pr-comment-stream" then
    return facts and facts.current_pr and facts.current_pr.comments or nil
  end
  return facts and facts.current and facts.current.comments or nil
end

local function signal_version(row, state)
  local signal = row and row.liveness_contract and row.liveness_contract.signal
  return M.liveness_heartbeat_version(state and state.version, signal)
end

local function matching_live_defer_marker(row, state, facts)
  local family = marker_family(row and row.defer and row.defer.live_marker)
  if family == nil then
    return nil
  end
  local comments = live_defer_comments(row, facts)
  if type(comments) ~= "table" then
    return nil
  end
  local proposal_id = (facts and facts.proposal_id) or (state and state.proposal_id)
  local version = signal_version(row, state)
  local newest = nil
  for _, comment in ipairs(M._trusted_marker_comments(comments)) do
    local created_ms = comment_created_ms(comment)
    for marker in M._comment_body(comment):gmatch(marker_pattern(family)) do
      if marker_attr(marker, "proposal") == tostring(proposal_id)
        and marker_attr(marker, "version") == tostring(version or "") then
        if newest == nil or (created_ms ~= nil and newest.created_ms ~= nil and created_ms > newest.created_ms) then
          newest = {
            id = family .. ":v1:" .. tostring(proposal_id) .. ":" .. tostring(version or ""),
            family = family,
            marker = marker,
            comment_created_at = M._comment_created_at(comment),
            created_ms = created_ms,
            updated_ms = created_ms,
          }
        end
      end
    end
  end
  return newest
end

local function generation_key(row, state, eval)
  return M._dedup_key({
    "restart-liveness:v2",
    tostring((state and state.proposal_id) or ""),
    tostring(row and row.from_state or ""),
    tostring(row and row.liveness_class_id or ""),
    tostring(eval.epoch_source or ""),
    tostring(eval.generation_opened_by or ""),
    tostring(eval.epoch_ms or ""),
  })
end

local function with_generation_key(row, state, eval)
  eval.generation_key = generation_key(row, state, eval)
  return eval
end

local function actionable(row, state, epoch_ms, opened_by, reason)
  return with_generation_key(row, state, {
    status = "actionable",
    epoch_ms = epoch_ms,
    epoch_source = row.actionable_epoch.source,
    generation_opened_by = opened_by,
    reason = reason,
  })
end

local function deferred(reason)
  return {
    status = "deferred",
    reason = reason,
  }
end

local function invalid(reason)
  return {
    status = "contract_invalid",
    reason = reason,
  }
end

local function clear_fact(row, state, facts)
  local comments = live_defer_comments(row, facts)
  local proposal_id = (facts and facts.proposal_id) or (state and state.proposal_id)
  local version = signal_version(row, state)
  if row and row.defer and row.defer.clear_fact == "dependency-release:v1" then
    local fact = M.dependency_release_fact(comments, proposal_id, version)
    if fact ~= nil then
      fact.id = "dependency-release:v1:" .. tostring(proposal_id) .. ":" .. tostring(version or "")
    end
    return fact
  end
  return nil
end

local function observed_fact(row, state, facts)
  local comments = live_defer_comments(row, facts)
  local proposal_id = (facts and facts.proposal_id) or (state and state.proposal_id)
  if row and row.defer and row.defer.observed_fact == "dependency-wait-observed:v1" then
    local fact = M.dependency_hold_fact(comments, proposal_id)
    if fact ~= nil then
      fact.id = "dependency-wait-observed:v1:" .. tostring(proposal_id) .. ":" .. tostring(fact.version or "")
    end
    return fact
  end
  return nil
end

local function dependency_gate_fact(row, state, facts)
  if row == nil
    or row.actionable_epoch == nil
    or row.actionable_epoch.allows_state_entry_if_never_deferred ~= true
    or row.defer == nil
    or row.defer.observed_fact ~= "dependency-wait-observed:v1" then
    return nil, "unsupported-never-deferred-signal"
  end
  if type(facts) == "table" and type(facts.dependency_gate) == "table" then
    return facts.dependency_gate, nil
  end
  return nil, "dependency-gate-missing"
end

local function fact_created_ms(fact)
  local seconds = M.iso_timestamp_epoch_seconds(fact and (fact.comment_created_at or fact.created_at))
  if seconds == nil then
    return nil
  end
  return seconds * 1000
end

local function resolve_state_entry(row, state)
  local epoch_ms = state_entry_ms(state)
  if epoch_ms == nil then
    return invalid("state entry epoch is missing")
  end
  return actionable(row, state, epoch_ms, "state-entry:v1:" .. tostring(state and state.version or ""), "state entry")
end

local function resolve_live_defer_epoch(row, state, facts, now_seconds)
  local clear = clear_fact(row, state, facts)
  local clear_ms = fact_created_ms(clear)
  local live = matching_live_defer_marker(row, state, facts)
  if live ~= nil and clear_ms ~= nil and live.updated_ms ~= nil and live.updated_ms <= clear_ms then
    live = nil
  end
  local freshness_ms = tonumber(row and row.defer and row.defer.freshness_ms)
  local now_ms = tonumber(now_seconds) and tonumber(now_seconds) * 1000 or nil
  if live ~= nil and live.updated_ms ~= nil and freshness_ms ~= nil and now_ms ~= nil then
    local stale_at = live.updated_ms + freshness_ms
    if stale_at > now_ms then
      return deferred("live defer marker fresh")
    end
    return actionable(row, state, stale_at, tostring(live.id) .. ":stale", "live defer marker stale")
  end
  if clear ~= nil and clear_ms ~= nil then
    return actionable(row, state, clear_ms, tostring(clear.id or "clear-fact"), "live defer clear fact")
  end
  local observed = observed_fact(row, state, facts)
  if observed == nil and row.actionable_epoch.allows_state_entry_if_never_deferred == true then
    local gate, gate_error = dependency_gate_fact(row, state, facts)
    if type(gate) ~= "table" then
      return invalid("live-defer-never-deferred-proof-missing:" .. tostring(gate_error or "dependency-gate-missing"))
    end
    if gate.ok == true then
      return resolve_state_entry(row, state)
    end
    return invalid("live-defer-clear-absent-after-dependency-gate:" .. tostring(gate.reason or gate.kind or "dependency-held"))
  end
  return invalid("live-defer marker absent but no durable clear fact or never-deferred proof exists")
end

function M.actionable_epoch_generation_key(row, state, eval)
  if type(eval) ~= "table" or eval.status ~= "actionable" then
    return nil
  end
  return generation_key(row, state, eval)
end

function M.actionable_epoch_resolve(row, state, facts, now_seconds)
  if type(row) ~= "table" or type(row.actionable_epoch) ~= "table" then
    return invalid("row does not declare actionable_epoch")
  end
  local sources = M.restart_liveness_epoch_sources()
  if sources[row.actionable_epoch.source] == nil then
    return invalid("unregistered actionable_epoch.source")
  end
  if row.actionable_epoch.source == "state_entry:v1" then
    return resolve_state_entry(row, state)
  end
  if row.actionable_epoch.source == "live_defer_epoch:v1" then
    return resolve_live_defer_epoch(row, state, facts, now_seconds)
  end
  return invalid("unsupported actionable_epoch.source")
end

function M.actionable_epoch_timeout_due(row, state, facts, now_seconds)
  local eval = M.actionable_epoch_resolve(row, state, facts, now_seconds)
  if type(facts) == "table" then
    facts.actionable_epoch_eval = eval
  end
  if eval.status ~= "actionable" then
    return false, nil
  end
  local now_ms = tonumber(now_seconds) and tonumber(now_seconds) * 1000 or nil
  local epoch_ms = tonumber(eval.epoch_ms)
  if now_ms == nil or epoch_ms == nil or now_ms < epoch_ms then
    return false, nil
  end
  local age = math.floor((now_ms - epoch_ms) / 60000)
  local budget = row.budget and tonumber(row.budget.minutes) or nil
  if budget == nil or age < budget then
    return false, age
  end
  return true, age
end

function M.actionable_epoch_timeout_attempt(row, state, facts)
  local eval = facts and facts.actionable_epoch_eval
  if type(eval) ~= "table" or eval.status ~= "actionable" or eval.generation_key == nil then
    return 0
  end
  local comments = facts and facts.current and facts.current.comments or nil
  local proposal_id = (facts and facts.proposal_id) or (state and state.proposal_id)
  local current = M.timeout_attempt_v2_round(comments, proposal_id, row, eval.generation_key)
  if tostring(eval.generation_opened_by or ""):find("^state%-entry:v1:") then
    return math.max(current, M.timeout_attempt_round(comments, proposal_id, state and state.version, row and row.from_state) or 0, M.version_timeout_round(state and state.version, row and row.from_state) or 0)
  end
  return current
end

function M.restart_row_has_registered_actionable_epoch(row)
  if type(row) ~= "table" or type(row.actionable_epoch) ~= "table" then
    return false
  end
  return row.actionable_epoch.source == "live_defer_epoch:v1"
    and M.restart_liveness_epoch_sources()[row.actionable_epoch.source] ~= nil
end

end

return S
