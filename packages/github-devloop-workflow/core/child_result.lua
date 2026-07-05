local devloop_marker_facts = require("devloop.markers.facts")

local M = {}

M.STATUS_RESULT_READY = "result_ready"
M.STATUS_FATAL = "fatal"
M.STATUS_RECOVERABLE = "recoverable"
M.STATUS_RUNNING = "running"
M.STATUS_UNKNOWN = "unknown"

local function call_reader(fn, ...)
  if type(fn) ~= "function" then
    return nil, false
  end
  local ok, result = pcall(fn, ...)
  if not ok then
    return nil, false
  end
  return result, true
end

local function truthy(value)
  return value == true or (type(value) == "table" and value.ok == true)
end

local function has_trusted_merged_marker(deps, child_ref)
  local direct, ok = call_reader(deps.has_merged_marker, child_ref)
  if ok and truthy(direct) then
    return true
  end
  if not ok then
    return nil
  end

  local entity, entity_ok = call_reader(deps.current_entity, child_ref)
  if not entity_ok then
    return type(deps.current_entity) == "function" and nil or false
  end
  if type(entity) ~= "table" or type(entity.comments) ~= "table" then
    return false
  end

  local proposal_id = entity.proposal_id or child_ref.proposal_id
  local pr_number = entity.pr_number or child_ref.pr_number
  if proposal_id == nil then
    return false
  end
  if pr_number == nil then
    return false
  end
  return devloop_marker_facts.merged_fact(entity.comments, proposal_id, pr_number, entity.version) ~= nil
end

local function github_closed_with_merged_pr(deps, child_ref)
  local fact, ok = call_reader(deps.github_closed_with_merged_pr, child_ref)
  if not ok then
    return nil
  end
  return truthy(fact)
end

local function has_irreversible_terminal_fact(deps, child_ref)
  local fact, ok = call_reader(deps.irreversible_terminal, child_ref)
  if not ok then
    return nil
  end
  if truthy(fact) then
    return true
  end
  return false
end

local function has_recoverable_fact(deps, child_ref)
  local recovery, recovery_ok = call_reader(deps.recovery_in_progress, child_ref)
  if recovery_ok and truthy(recovery) then
    return true
  end
  if not recovery_ok then
    return nil
  end

  local retryable, retryable_ok = call_reader(deps.impl_failed_retryable, child_ref)
  if retryable_ok and truthy(retryable) then
    return true
  end
  if not retryable_ok then
    return nil
  end
  return false
end

local function impl_failed_is_fatal(deps, child_ref)
  local non_retryable, non_retryable_ok = call_reader(deps.impl_failed_non_retryable, child_ref)
  if non_retryable_ok and truthy(non_retryable) then
    return true
  end
  if not non_retryable_ok then
    if type(deps.impl_failed_non_retryable) == "function" then
      return nil
    end
  end
  return false
end

local function impl_failed_reason(deps, child_ref)
  local reason, ok = call_reader(deps.impl_failed_reason, child_ref)
  if not ok then
    if type(deps.impl_failed_reason) == "function" then
      return nil, false
    end
    return nil, true
  end
  if reason == nil then
    return nil, true
  end
  return tostring(reason), true
end

-- Uses only exact child-boundary evidence:
--   deps.has_merged_marker
--   devloop.markers.facts.merged_fact over trusted marker comments
-- and GitHub-native child boundary readers injected as deps. It does not
-- require peer package internals and never enumerates private routing states.
function M.child_result_status(deps, child_ref)
  if type(deps) ~= "table" or type(child_ref) ~= "table" then
    return M.STATUS_UNKNOWN
  end

  local merged_marker = has_trusted_merged_marker(deps, child_ref)
  if merged_marker == nil then
    return M.STATUS_UNKNOWN
  end
  if merged_marker then
    return M.STATUS_RESULT_READY, { merged = true }
  end

  local native_merged = github_closed_with_merged_pr(deps, child_ref)
  if native_merged == nil then
    return M.STATUS_UNKNOWN
  end
  if native_merged then
    return M.STATUS_RESULT_READY, { merged = true }
  end

  local impl_failed_fatal = impl_failed_is_fatal(deps, child_ref)
  if impl_failed_fatal == nil then
    return M.STATUS_UNKNOWN
  end
  if impl_failed_fatal then
    local reason, reason_ok = impl_failed_reason(deps, child_ref)
    if not reason_ok then
      return M.STATUS_UNKNOWN
    end
    return M.STATUS_FATAL, { impl_failed_reason = reason }
  end

  local fatal = has_irreversible_terminal_fact(deps, child_ref)
  if fatal == nil then
    return M.STATUS_UNKNOWN
  end
  if fatal then
    return M.STATUS_FATAL
  end

  local recoverable = has_recoverable_fact(deps, child_ref)
  if recoverable == nil then
    return M.STATUS_UNKNOWN
  end
  if recoverable then
    return M.STATUS_RECOVERABLE
  end

  return M.STATUS_RUNNING
end

function M.install(target)
  target.child_result = M
  target.child_result_status = M.child_result_status
end

return M
