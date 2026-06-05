local core = require("core")
local mapping = require("departments.propose.mapping")

local M = {}

M.spec = {
  consumes = { "issue" },
  produces = { "consensus.proposal" },
  stall_window = "30s",
}

function pipeline(event)
  local issue = event.payload or {}
  if issue.schema ~= "autochrono.issue.v1" then
    log.warn("autochrono: unsupported issue schema")
    return
  end
  if not core.is_eligible(issue) then
    return
  end

  local cache_key = core.proposal_cache_key(issue.repo, issue.issue_number, issue.updated_at)
  with_lock(cache_key, function()
    if cache_get(cache_key) then
      return
    end

    local ok, proposal = pcall(mapping.build_proposal, issue)
    -- Fail closed: never raise a proposal consensus would reject, and never cache it
    -- (that would silence this issue forever).
    if not ok or not core.validate_proposal(proposal) then
      log.warn("autochrono: cannot build a valid proposal; skipping")
      return
    end

    raise("consensus.proposal", proposal)
    cache_set(cache_key, core.proposal_id(issue.repo, issue.issue_number))
  end)
end

return M
