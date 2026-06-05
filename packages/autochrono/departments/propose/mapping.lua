local core = require("core")

local M = {}

local function proposal_title(issue)
  return core.bounded_text("Draft maintainer reply for issue #" .. tostring(issue.issue_number), 240)
end

local function proposal_body(issue)
  local fields = core.require_issue_fields(issue)
  return core.bounded_text(table.concat({
    "Draft a concise GitHub issue reply for the fkst autochrono package.",
    "",
    "Use a calm maintainer voice.",
    "Do not claim work has been completed.",
    "Do not include markdown headings.",
    "Keep the reply under 120 words.",
    "",
    "Issue:",
    "Repository: " .. tostring(fields.repo),
    "Number: " .. tostring(fields.issue_number),
    "Title: " .. tostring(fields.title),
    "URL: " .. tostring(fields.url),
    "Updated at: " .. tostring(fields.updated_at),
  }, "\n"), core.max_body_len())
end

function M.build_proposal(issue)
  local fields = core.require_issue_fields(issue)
  local proposal_id = core.proposal_id(fields.repo, fields.issue_number)

  return {
    schema = "consensus.proposal.v1",
    proposal_id = proposal_id,
    dedup_key = core.proposal_dedup_key(fields.repo, fields.issue_number, fields.updated_at),
    title = proposal_title(fields),
    body = proposal_body(issue),
    source_ref = fields.source_ref,
  }
end

return M
