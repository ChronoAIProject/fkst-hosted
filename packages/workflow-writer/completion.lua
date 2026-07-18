-- workflow-writer: the COMPLETION seam (child-status reader).
--
-- This is a FRESH, pure, side-effect-free reader (NOT ported from the dev
-- child_result.lua, which is coupled to devloop.markers.facts). It is the third
-- argument workflow.engine.frontier.compute_frontier consumes: given the child_ref of a
-- materialized authoring step, it returns one of the five whitelisted statuses. Any
-- other/thrown value is coerced to "unknown" by the frontier, so this reader only ever
-- needs to return the mapped status.
--
-- The authoring step's child is a pull request that adds/edits a template file. Its
-- durable state is resolved by the discovery seam (via a PR read) and attached to the
-- child_ref as `result`:
--   result.state == "merged"    -> result_ready  (PR merged; the template landed)
--   result.state == "open"      -> running       (PR open, awaiting review/merge)
--   result.state == "transient" -> recoverable   (a transient read/spawn failure; retry)
--   result.state == "invalid"   -> fatal         (draft failed validation / PR closed unmerged)
--   anything else / missing     -> unknown        (never advances, never terminalizes)
local M = {}

local PR_STATE_TO_STATUS = {
  merged = "result_ready",
  open = "running",
  transient = "recoverable",
  invalid = "fatal",
}

-- Map one durable PR-state descriptor to the frontier status enum. Pure. Distinct from
-- the security reader: this one is keyed on pull-request lifecycle, not analysis output.
function M.status_of_pr(result)
  if type(result) ~= "table" then
    return "unknown"
  end
  local mapped = PR_STATE_TO_STATUS[tostring(result.state or "")]
  if mapped == nil then
    return "unknown"
  end
  return mapped
end

-- Build the child_status_of function for a scope. The reader is pure over the child_ref
-- the frontier hands back from the created materialization facts; the discovery seam is
-- responsible for attaching the durable `result` descriptor (the PR lifecycle state).
function M.reader(_scope)
  local function child_status_of(child_ref)
    if type(child_ref) ~= "table" then
      return "unknown"
    end
    local status = M.status_of_pr(child_ref.result)
    return status, child_ref.result
  end
  return child_status_of
end

return M
