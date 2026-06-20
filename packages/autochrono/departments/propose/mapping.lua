local core = require("core")

local M = {}

local function proposal_title(issue)
  return core.bounded_text("Draft maintainer reply for issue #" .. tostring(issue.issue_number), 240)
end

local function proposal_body(issue)
  local fields = core.require_issue_fields(issue)
  local prompt = require("prompts.proposal")
  local rendered = core.render_template(prompt.template, {
    repo = fields.repo,
    issue_number = fields.issue_number,
    title = fields.title,
    url = fields.url,
    updated_at = fields.updated_at,
  })
  return rendered
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
    content_fetch = core.content_fetch_manifest(fields.source_ref),
  }
end

return M
