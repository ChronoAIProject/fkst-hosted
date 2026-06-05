local M = {}

local default_stall_window = "2m"

local function trim(value)
  return tostring(value or ""):gsub("^%s+", ""):gsub("%s+$", "")
end

local function require_field(issue, name)
  local value = issue[name]
  if value == nil or value == "" then
    error("autochrono: missing " .. name)
  end
  return value
end

function M.reply_dedup_key(repo, issue_number)
  return "autochrono:" .. tostring(repo) .. "#issue/" .. tostring(issue_number)
end

function M.replied_cache_key(repo, issue_number)
  return "autochrono/replied/" .. tostring(repo) .. "/issue/" .. tostring(issue_number)
end

function M.is_eligible(issue)
  if type(issue) ~= "table" then
    return false
  end
  if issue.schema ~= "autochrono.issue.v1" then
    return false
  end
  if issue.repo == nil or issue.issue_number == nil then
    return false
  end
  return issue.state == "OPEN"
end

function M.build_prompt(issue)
  if type(issue) ~= "table" then
    error("autochrono: issue must be a table")
  end

  local repo = require_field(issue, "repo")
  local issue_number = require_field(issue, "issue_number")
  local title = require_field(issue, "title")
  local url = require_field(issue, "url")
  local updated_at = require_field(issue, "updated_at")

  return table.concat({
    "Draft a concise GitHub issue reply for the fkst autochrono package.",
    "",
    "Use a calm maintainer voice.",
    "Do not claim work has been completed.",
    "Do not include markdown headings.",
    "Keep the reply under 120 words.",
    "",
    "Issue:",
    "Repository: " .. tostring(repo),
    "Number: " .. tostring(issue_number),
    "Title: " .. tostring(title),
    "URL: " .. tostring(url),
    "Updated at: " .. tostring(updated_at),
  }, "\n")
end

function M.clean_draft(stdout)
  local body = trim(stdout)
  if body == "" then
    return nil
  end
  return body
end

function M.build_reply_request(issue, body)
  if type(issue) ~= "table" then
    error("autochrono: issue must be a table")
  end

  local repo = require_field(issue, "repo")
  local issue_number = require_field(issue, "issue_number")
  local reply_body = require_field({ body = body }, "body")
  -- source_ref is required: the reply flows to a reliable downstream that
  -- fails closed without it, so reject early rather than emit an
  -- unrecoverable reply request.
  local source_ref = require_field(issue, "source_ref")

  return {
    schema = "autochrono.reply.v1",
    repo = repo,
    issue_number = issue_number,
    body = reply_body,
    dedup_key = M.reply_dedup_key(repo, issue_number),
    source_ref = source_ref,
  }
end

function M.draft_reply(issue)
  local result = spawn_codex_sync({
    prompt = M.build_prompt(issue),
    stall_window = default_stall_window,
  })

  if type(result) ~= "table" or result.exit_code ~= 0 then
    return nil
  end
  return M.clean_draft(result.stdout)
end

return M
