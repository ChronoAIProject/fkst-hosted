local core = require("core")

local M = {}

M.spec = {
  consumes = { "issue" },
  produces = { "reply" },
  stall_window = "2m",
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

  local key = core.replied_cache_key(issue.repo, issue.issue_number)
  local dedup_key = core.reply_dedup_key(issue.repo, issue.issue_number)
  with_lock(key, function()
    if cache_get(key) then
      return
    end

    local body = core.draft_reply(issue)
    if body == nil then
      return
    end

    raise("reply", {
      schema = "autochrono.reply.v1",
      repo = issue.repo,
      issue_number = issue.issue_number,
      body = body,
      dedup_key = dedup_key,
      source_ref = issue.source_ref,
    })
    cache_set(key, dedup_key)
  end)
end

return M
