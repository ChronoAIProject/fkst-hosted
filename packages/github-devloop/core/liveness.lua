local S = {}

function S.install(M)
local max_timeout_attempts = 3

local function has_required_table(row, field)
  return type(row[field]) == "table" and next(row[field]) ~= nil
end

local function valid_budget(row)
  return type(row.budget) == "table"
    and tonumber(row.budget.minutes) ~= nil
    and tonumber(row.budget.minutes) > 0
end

local function valid_timeout(row)
  if type(row.on_timeout) ~= "table" then
    return false
  end
  if row.on_timeout.action ~= "redrive" or row.on_timeout.queue ~= row.driving_queue then
    return false
  end
  return tonumber(row.on_timeout.escalate_after_attempts) ~= nil
    and tonumber(row.on_timeout.escalate_after_attempts) > 0
end

function M.liveness_contract_errors(rows)
  local errors = {}
  for _, row in ipairs(rows or M.restart_transition_table()) do
    if type(row.from_state) ~= "string" or row.from_state == "" then
      table.insert(errors, "row: missing from_state")
    end
    if type(row.terminal) ~= "boolean" then
      table.insert(errors, tostring(row.from_state or "?") .. ": terminal must be boolean")
    end
    if row.terminal == true then
      if row.output_obligation ~= nil then
        table.insert(errors, tostring(row.from_state or "?") .. ": terminal row must not declare output_obligation")
      end
    else
      if not has_required_table(row, "output_obligation") then
        table.insert(errors, tostring(row.from_state or "?") .. ": non-terminal row must declare output_obligation")
      end
      if not valid_budget(row) then
        table.insert(errors, tostring(row.from_state or "?") .. ": non-terminal row must declare a positive budget")
      end
      if not valid_timeout(row) then
        table.insert(errors, tostring(row.from_state or "?") .. ": non-terminal row must declare redrive on_timeout for its driving queue")
      end
      if (type(row.to_states) ~= "table" or #row.to_states == 0)
        and (type(row.reentry_commands) ~= "table" or #row.reentry_commands == 0) then
        table.insert(errors, tostring(row.from_state or "?") .. ": non-terminal row must declare at least one next state")
      end
    end
    for _, next_state in ipairs(row.to_states or {}) do
      if M._label_by_state[next_state] == nil then
        table.insert(errors, tostring(row.from_state or "?") .. ": unknown next state " .. tostring(next_state))
      end
    end
  end
  return errors
end

function M.liveness_terminal_states(rows)
  local terminals = {}
  for _, row in ipairs(rows or M.restart_transition_table()) do
    if row.terminal == true then
      table.insert(terminals, row.from_state)
    end
  end
  return terminals
end

function M.liveness_budget_minutes(state_name)
  local row = M.restart_transition_row(state_name)
  return row and row.budget and tonumber(row.budget.minutes) or nil
end

function M.liveness_state_age_minutes(state, now_seconds)
  if type(state) ~= "table" then
    return nil
  end
  if state.marker_created_at ~= nil and state.marker_created_at ~= "" then
    local created_seconds = M.iso_timestamp_epoch_seconds(state.marker_created_at)
    local current_seconds = tonumber(now_seconds)
    if created_seconds ~= nil and current_seconds ~= nil and current_seconds >= created_seconds then
      return math.floor((current_seconds - created_seconds) / 60)
    end
  end
  return M.stall_suspect_age_minutes(state.version, now_seconds)
end

function M.liveness_timeout_attempt(row, state)
  return M.version_timeout_round(state and state.version, row and row.from_state)
end

function M.next_liveness_timeout_version(row, state)
  local base = tostring(state and state.version or "")
  return base .. "/timeout/" .. tostring(row.from_state) .. "/" .. tostring(M.liveness_timeout_attempt(row, state) + 1)
end

function M.liveness_timeout_due(row, state, now_seconds)
  if row == nil or row.terminal == true then
    return false, nil
  end
  local budget = row.budget and tonumber(row.budget.minutes) or nil
  local age = M.liveness_state_age_minutes(state, now_seconds)
  if budget == nil or age == nil or age < budget then
    return false, age
  end
  return true, age
end

local function timeout_escalation(row, state, age)
  local attempt = M.liveness_timeout_attempt(row, state)
  local limit = tonumber(row.on_timeout and row.on_timeout.escalate_after_attempts) or max_timeout_attempts
  if attempt >= limit then
    return {
      action = "escalate",
      attempt = attempt,
      age_minutes = age,
    }
  end
  return {
    action = "redrive",
    attempt = attempt + 1,
    age_minutes = age,
    version = M.next_liveness_timeout_version(row, state),
  }
end

local function build_timeout_reconcile(row, entity, state, facts, decision)
  local source_ref = (facts and facts.source_ref) or (entity and entity.source_ref) or (state and state.source_ref)
  if row.from_state == "thinking" and M._has_bounded_source_ref(source_ref) then
    local base_version = M.strip_transition_version_suffixes(state.version)
    return "devloop_reconcile", M.build_devloop_reconcile_payload({
      proposal_id = state.proposal_id,
      source_ref = source_ref,
    }, decision.attempt, base_version)
  end
  if row.from_state == "reviewing"
    and M._has_bounded_source_ref(source_ref)
    and M._is_git_sha(facts and facts.head_sha) then
    local review_proposal_id = facts and facts.review_proposal_id
    if review_proposal_id == nil and entity ~= nil and entity.repo ~= nil then
      local _, pr_number = M.parse_pr_source_ref(source_ref)
      if pr_number ~= nil then
        review_proposal_id = M.pr_review_proposal_id(entity.repo, pr_number, state.version, facts.head_sha)
      end
    end
    if review_proposal_id ~= nil then
      return "devloop_review_reconcile", M.build_devloop_review_reconcile_payload({
        proposal_id = review_proposal_id,
        source_ref = source_ref,
      }, decision.attempt, state.proposal_id, M.safe_version_segment(state.version), facts.head_sha)
    end
  end
  if M._has_bounded_source_ref(source_ref)
    and M._is_path_safe_key(state and state.proposal_id, M._max_key_len)
    and M._is_bounded_string(state and state.version, M._max_dedup_len) then
    return "devloop_timeout_reconcile", M.build_devloop_timeout_reconcile_payload(row, state, state.proposal_id, source_ref, decision.attempt)
  end
  return nil, nil
end

function M.liveness_timeout_decision(row, state, now_seconds)
  local due, age = M.liveness_timeout_due(row, state, now_seconds)
  if not due then
    return {
      action = "wait",
      age_minutes = age,
    }
  end
  return timeout_escalation(row, state, age)
end

function M.maybe_timeout_redrive_from_table(dept, entity, state, table_row, facts)
  local row = table_row or M.restart_transition_row(state and state.state)
  if row == nil or row.terminal == true then
    return false
  end
  local decision = M.liveness_timeout_decision(row, state, (facts and facts.now_seconds) or now())
  local proposal_id = facts and facts.proposal_id or state and state.proposal_id
  if decision.action == "wait" then
    return false
  end
  M.log_cas_decision(dept, proposal_id, state, row.from_state, row.driving_queue, "timeout-" .. decision.action, "state output obligation exceeded budget")
  if decision.action == "escalate" then
    local queue, payload = build_timeout_reconcile(row, entity, state, facts, decision)
    if queue ~= nil then
      M.log_apply(dept, proposal_id, nil, nil, { add = {}, remove = {} }, { queue })
      M.log_raise(dept, proposal_id, queue, payload)
      return true
    end
    return false
  end
  return M.replay_from_table(dept, entity, {
    state = state.state,
    version = decision.version or M.next_liveness_timeout_version(row, state),
    proposal_id = state.proposal_id,
    stage_rank = state.stage_rank,
    marker_created_at = state.marker_created_at,
  }, row, facts)
end

end

return S
