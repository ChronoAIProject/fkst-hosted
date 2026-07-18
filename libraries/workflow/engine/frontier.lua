local blueprint = require("workflow.engine.blueprint")
local marker = require("workflow.engine.marker")
local strings = require("contract.strings")

local M = {}

local valid_status = {
  result_ready = true,
  fatal = true,
  recoverable = true,
  running = true,
  unknown = true,
}

local function terminal_error(reason)
  return {
    action = "terminal",
    state = "error",
    reason_code = reason,
  }
end

local function terminal_blocked(reason, slot, child_ref)
  return {
    action = "terminal",
    state = "blocked",
    reason_code = reason,
    slot = slot,
    child_ref = child_ref,
  }
end

local function terminal_block_reason(base, slot, detail)
  local parts = { tostring(base or "child-fatal") }
  if slot ~= nil and tostring(slot) ~= "" then
    parts[#parts + 1] = tostring(slot)
  end
  if type(detail) == "table"
    and type(detail.impl_failed_reason) == "string"
    and detail.impl_failed_reason ~= "" then
    parts[#parts + 1] = detail.impl_failed_reason
  end
  local reason = strings.sanitize_key(table.concat(parts, "-"), marker.MAX_TERMINAL_REASON_CODE_BYTES)
    :gsub("/", "-")
    :gsub("%-+", "-")
    :gsub("^%-+", "")
    :gsub("%-+$", "")
  if reason == "" then
    return tostring(base or "child-fatal")
  end
  return reason
end

local function wait(why, slot, child_ref)
  return {
    action = "wait",
    why = why,
    slot = slot,
    child_ref = child_ref,
  }
end

local function created_entry_for_slot(by_slot, slot_id)
  local entry = by_slot[tostring(slot_id)]
  if type(entry) == "table" and entry.state == "created" then
    if entry.child_ref ~= nil then
      return entry.child_ref, entry
    end
    if entry.child_issue ~= nil then
      return {
        kind = "issue",
        issue_number = entry.child_issue,
        proposal_id = entry.child_proposal_id,
        source_ref = entry.child_source_ref,
      }, entry
    end
  end
  return nil, entry
end

local function normalize_ledger(ledger_facts)
  if type(ledger_facts) ~= "table" then
    return nil
  end
  if ledger_facts.by_slot ~= nil then
    return ledger_facts.by_slot
  end
  if ledger_facts[1] ~= nil then
    return marker.latest_materialization_by_slot(ledger_facts)
  end
  return ledger_facts
end

local function impossible_ledger(by_slot, step_ids, steps)
  for slot_id, entry in pairs(by_slot) do
    if step_ids[tostring(slot_id)] ~= true then
      return true
    end
    if type(entry) ~= "table" or marker.MATERIALIZATION_STATES[entry.state] ~= true then
      return true
    end
    if entry.state == "created" and entry.child_issue == nil and entry.child_ref == nil then
      return true
    end
  end
  for index, step in ipairs(steps) do
    local entry = by_slot[tostring(step.id)]
    if type(entry) == "table" and entry.state == "created" and index > 1 then
      local predecessor = by_slot[tostring(steps[index - 1].id)]
      if type(predecessor) ~= "table" or predecessor.state ~= "created" then
        return true
      end
    end
  end
  return false
end

local function child_status(child_status_of, child_ref)
  local ok, status, detail = pcall(child_status_of, child_ref)
  if not ok or valid_status[status] ~= true then
    return "unknown", nil
  end
  return status, detail
end

function M.compute_frontier(plan, ledger_facts, child_status_of)
  if type(child_status_of) ~= "function" then
    return terminal_error("missing-child-status-reader")
  end
  local ok = blueprint.validate(plan)
  if not ok then
    return terminal_error("corrupt-blueprint")
  end

  local by_slot = normalize_ledger(ledger_facts)
  if type(by_slot) ~= "table" then
    return terminal_error("corrupt-ledger")
  end

  local step_ids = {}
  for _, step in ipairs(plan.steps) do
    step_ids[tostring(step.id)] = true
  end
  if impossible_ledger(by_slot, step_ids, plan.steps) then
    return terminal_error("impossible-ledger")
  end

  local created = {}
  local child_refs = {}
  local child_statuses = {}
  for index, step in ipairs(plan.steps) do
    local child_ref, entry = created_entry_for_slot(by_slot, step.id)
    if child_ref ~= nil then
      created[index] = true
      child_refs[index] = child_ref
      local status, detail = child_status(child_status_of, child_ref)
      child_statuses[index] = status
      if status == "fatal" then
        return terminal_blocked(terminal_block_reason("child-fatal", step.id, detail), step.id, child_ref)
      end
      if status == "recoverable" then
        return wait("child-recoverable", step.id, child_ref)
      end
    end
  end

  local all_ready = true
  for index, step in ipairs(plan.steps) do
    if created[index] then
      local status = child_statuses[index]
      if status ~= "result_ready" then
        all_ready = false
      end
    else
      all_ready = false
      local predecessor = nil
      if index > 1 then
        predecessor = child_refs[index - 1]
        if predecessor == nil then
          return wait("predecessor-not-created", step.id, nil)
        end
        local predecessor_status = child_statuses[index - 1] or child_status(child_status_of, predecessor)
        if predecessor_status == "result_ready" then
          return {
            action = "materialize",
            slot = step.id,
            predecessor = predecessor,
          }
        end
        if predecessor_status == "fatal" then
          return terminal_blocked("predecessor-fatal", plan.steps[index - 1].id, predecessor)
        end
        if predecessor_status == "recoverable" then
          return wait("predecessor-recoverable", plan.steps[index - 1].id, predecessor)
        end
        return wait("predecessor-" .. predecessor_status, plan.steps[index - 1].id, predecessor)
      end
      return {
        action = "materialize",
        slot = step.id,
        predecessor = nil,
      }
    end
  end

  if all_ready then
    return {
      action = "terminal",
      state = "done",
      reason_code = "all-slots-result-ready",
    }
  end
  return wait("children-not-yet-ready", nil, nil)
end

function M.install(target)
  target.frontier = M
  target.compute_frontier = M.compute_frontier
end

return M
