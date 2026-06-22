local core = require("core")
local mapping = require("departments.propose.mapping")
local saga = require("workflow.saga")

local spec = {
  consumes = { "issue" },
  published_seam = { "issue" },
  produces = { "consensus.proposal" },
  published_seam = { "issue" },
  stall_window = "30s",
}

local function proposal_done(event)
  local issue = event.payload or {}
  if issue.schema ~= "autochrono.issue.v1" then
    log.warn("autochrono: unsupported issue schema")
    return true
  end
  if not core.is_eligible(issue) then
    return true
  end

  local cache_key = core.proposal_cache_key(issue.repo, issue.issue_number, issue.updated_at)
  local already_proposed = false
  with_lock(cache_key, function()
    already_proposed = cache_get(cache_key) ~= nil
  end)
  return already_proposed
end

local function act_propose(event)
  local issue = event.payload or {}
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

return saga.department(spec, {
  done = proposal_done,
  act = act_propose,
  wrap = core.wrap_pipeline_failure,
  name = "propose",
})
