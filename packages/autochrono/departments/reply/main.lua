local core = require("core")
local mapping = require("departments.reply.mapping")

local M = {}

M.spec = {
  consumes = { "consensus.consensus_reached" },
  produces = { "reply" },
  stall_window = "30s",
}

function pipeline(event)
  local payload = event.payload or {}
  if payload.schema ~= "consensus.consensus_reached.v1" then
    log.warn("autochrono: unsupported consensus schema")
    return
  end

  local repo, issue_number = core.parse_proposal_id(payload.proposal_id)
  if repo == nil then
    return
  end
  if payload.decision ~= "approve" then
    return
  end
  -- Fail closed: a malformed consensus_reached must not yield an empty reply nor mark the
  -- issue replied (which would skip a later well-formed event).
  if not core.validate_reached(payload) then
    log.warn("autochrono: malformed consensus_reached; skipping reply")
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

return M
