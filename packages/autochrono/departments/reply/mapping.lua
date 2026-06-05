local core = require("core")

local M = {}

function M.build_reply(reached, repo, issue_number)
  if type(reached) ~= "table" then
    error("autochrono: consensus_reached must be a table")
  end

  return {
    schema = "autochrono.reply.v1",
    repo = repo,
    issue_number = issue_number,
    body = reached.body,
    dedup_key = core.reply_dedup_key(repo, issue_number),
    source_ref = core.normalize_source_ref(reached.source_ref),
  }
end

return M
