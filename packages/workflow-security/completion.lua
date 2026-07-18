-- workflow-security: the COMPLETION seam (child-status reader).
--
-- This is a FRESH, pure, side-effect-free reader (NOT ported from the dev
-- child_result.lua, which is coupled to devloop.markers.facts). It is the third
-- argument workflow.engine.frontier.compute_frontier consumes: given the child_ref
-- of a materialized analysis step, it returns one of the five whitelisted statuses.
-- Any other/thrown value is coerced to "unknown" by the frontier, so this reader
-- only ever needs to return the mapped status.
--
-- Each review step's child is a codex analysis run whose durable result descriptor
-- is resolved by the discovery seam and attached to the child_ref as `result`:
--   result.state == "ready"      -> result_ready  (output present, well-formed, durable)
--   result.state == "running"    -> running       (run still in flight)
--   result.state == "transient"  -> recoverable    (nonzero/timeout; retry may succeed)
--   result.state == "malformed"  -> fatal          (output present but validation failed)
--   anything else / missing      -> unknown         (never advances, never terminalizes)
local M = {}

local STATE_TO_STATUS = {
  ready = "result_ready",
  running = "running",
  transient = "recoverable",
  malformed = "fatal",
}

-- Map one durable result descriptor to the frontier status enum. Pure.
function M.status_of_result(result)
  if type(result) ~= "table" then
    return "unknown"
  end
  return STATE_TO_STATUS[tostring(result.state or "")] or "unknown"
end

-- Build the child_status_of function for a scope. The reader is pure over the
-- child_ref the frontier hands back from the created materialization facts; the
-- discovery seam is responsible for attaching the durable `result` descriptor.
function M.reader(_scope)
  return function(child_ref)
    if type(child_ref) ~= "table" then
      return "unknown"
    end
    local status = M.status_of_result(child_ref.result)
    return status, child_ref.result
  end
end

return M
