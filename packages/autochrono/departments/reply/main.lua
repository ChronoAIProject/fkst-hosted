local core = require("core")
local mapping = require("departments.reply.mapping")
local saga = require("workflow.saga")

local spec = {
  consumes = { "consensus.consensus_reached" },
  produces = { "reply" },
  fanout = { "consensus.consensus_reached" },
  stall_window = "30s",
}

local function reply_done(event)
  local payload = event.payload or {}
  if payload.schema ~= "consensus.consensus_reached.v1" then
    log.warn("autochrono: unsupported consensus schema")
    return true
  end

  local repo, issue_number = core.parse_proposal_id(payload.proposal_id)
  if repo == nil then
    return true
  end
  if payload.decision ~= "approve" then
    return true
  end
  -- Fail closed: a malformed consensus_reached must not yield an empty reply nor mark the
  -- issue replied (which would skip a later well-formed event).
  if not core.validate_reached(payload) then
    log.warn("autochrono: malformed consensus_reached; skipping reply")
    return true
  end

  local cache_key = core.replied_cache_key(repo, issue_number)
  local already_replied = false
  with_lock(cache_key, function()
    already_replied = cache_get(cache_key) ~= nil
  end)
  return already_replied
end

local function act_reply(event)
  local payload = event.payload or {}
  local repo, issue_number = core.parse_proposal_id(payload.proposal_id)
  if repo == nil or payload.decision ~= "approve" or not core.validate_reached(payload) then
    return
  end

  local cache_key = core.replied_cache_key(repo, issue_number)
  with_lock(cache_key, function()
    if cache_get(cache_key) then
      return
    end

    raise("reply", mapping.build_reply(payload, repo, issue_number))
    cache_set(cache_key, core.reply_dedup_key(repo, issue_number))
  end)
end

return saga.department(spec, {
  done = reply_done,
  act = act_reply,
  wrap = core.wrap_pipeline_failure,
  name = "reply",
})
