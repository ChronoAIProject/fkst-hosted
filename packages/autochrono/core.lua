local M = {}

local max_key_len = 200
local max_title_len = 240
local max_body_len = 12000
local max_repo_key_len = 100
local max_issue_key_len = 30
local max_update_key_len = 50

local function is_bounded_string(value, limit)
  return type(value) == "string" and value ~= "" and #value <= limit
end

-- Mirrors consensus's is_path_safe_key so propose can fail closed BEFORE raising a
-- proposal the consensus engine would reject (and before wrongly writing its cache).
local function is_path_safe_key(value, limit)
  if not is_bounded_string(value, limit or max_key_len) then
    return false
  end
  if value:sub(1, 1) == "/" then
    return false
  end
  if value:find("\\", 1, true) ~= nil then
    return false
  end
  if value:find("%s") ~= nil then
    return false
  end
  if value:find("[^%w%._%-%/#]") ~= nil then
    return false
  end
  for segment in value:gmatch("[^/]+") do
    if segment == "." or segment == ".." then
      return false
    end
  end
  return true
end

local function has_bounded_source_ref(source_ref)
  return type(source_ref) == "table"
    and is_bounded_string(source_ref.kind, max_key_len)
    and is_bounded_string(source_ref.ref, max_key_len)
end

local function require_field(payload, name)
  local value = payload[name]
  if value == nil or value == "" then
    error("autochrono: missing " .. name)
  end
  return value
end

local function require_bounded_field(payload, name, limit)
  local value = require_field(payload, name)
  if not is_bounded_string(tostring(value), limit) then
    error("autochrono: invalid " .. name)
  end
  return value
end

function M.sanitize_key(value)
  local sanitized = tostring(value or ""):gsub("[^%w%._%-%/#]", "-")
  sanitized = sanitized:gsub("/+", "/")
  sanitized = sanitized:gsub("^/+", ""):gsub("/+$", "")
  if sanitized == "" then
    return "empty"
  end

  local segments = {}
  for segment in sanitized:gmatch("[^/]+") do
    if segment == "." or segment == ".." then
      segment = "-"
    end
    table.insert(segments, segment)
  end

  sanitized = table.concat(segments, "/")
  if #sanitized > max_key_len then
    sanitized = sanitized:sub(1, max_key_len)
    sanitized = sanitized:gsub("/+$", "")
  end
  if sanitized == "" then
    return "empty"
  end
  return sanitized
end

local function safe_repo(repo)
  return M.sanitize_key(repo):sub(1, max_repo_key_len):gsub("/+$", "")
end

local function safe_issue_number(issue_number)
  return M.sanitize_key(issue_number):sub(1, max_issue_key_len):gsub("/+$", "")
end

local function safe_updated_at(updated_at)
  return M.sanitize_key(updated_at):sub(1, max_update_key_len):gsub("/+$", "")
end

function M.reply_dedup_key(repo, issue_number)
  return "autochrono:" .. tostring(repo) .. "#issue/" .. tostring(issue_number)
end

function M.replied_cache_key(repo, issue_number)
  return "autochrono/replied/" .. safe_repo(repo) .. "/issue/" .. safe_issue_number(issue_number)
end

function M.proposal_id(repo, issue_number)
  return "autochrono/issue/" .. safe_repo(repo) .. "/" .. safe_issue_number(issue_number)
end

function M.parse_proposal_id(id)
  if type(id) ~= "string" then
    return nil
  end

  local rest = id:match("^autochrono/issue/(.+)$")
  if rest == nil then
    return nil
  end

  local issue_number = rest:match("/([^/]+)$")
  local repo = issue_number and rest:sub(1, #rest - #issue_number - 1) or nil
  if repo == nil or repo == "" or issue_number == nil or issue_number == "" then
    return nil
  end
  return repo, issue_number
end

function M.proposal_cache_key(repo, issue_number, updated_at)
  return "autochrono/proposed/v1/" .. safe_repo(repo)
    .. "/issue/" .. safe_issue_number(issue_number)
    .. "/updated/" .. safe_updated_at(updated_at)
end

-- Bounded dedup_key: proposal_id (<=148) + "/" + safe_updated_at (<=50) stays under the
-- consensus 200-char key cap, unlike the old proposal_id .. sanitize_key(updated_at).
function M.proposal_dedup_key(repo, issue_number, updated_at)
  return M.proposal_id(repo, issue_number) .. "/" .. safe_updated_at(updated_at)
end

function M.normalize_source_ref(source_ref)
  if type(source_ref) ~= "table" then
    error("autochrono: invalid source_ref")
  end
  if not is_bounded_string(source_ref.kind, max_key_len) then
    error("autochrono: invalid source_ref.kind")
  end
  if not is_bounded_string(source_ref.ref, max_key_len) then
    error("autochrono: invalid source_ref.ref")
  end
  return {
    kind = source_ref.kind,
    ref = source_ref.ref,
  }
end

function M.is_eligible(issue)
  if type(issue) ~= "table" then
    return false
  end
  if issue.schema ~= "autochrono.issue.v1" then
    return false
  end
  if issue.state ~= "OPEN" then
    return false
  end
  if issue.repo == nil or issue.issue_number == nil then
    return false
  end
  if issue.title == nil or issue.url == nil or issue.updated_at == nil then
    return false
  end
  if type(issue.source_ref) ~= "table" or issue.source_ref.kind == nil or issue.source_ref.ref == nil then
    return false
  end
  return true
end

function M.require_issue_fields(issue)
  if type(issue) ~= "table" then
    error("autochrono: issue must be a table")
  end
  return {
    repo = require_bounded_field(issue, "repo", max_key_len),
    issue_number = require_bounded_field(issue, "issue_number", max_key_len),
    title = require_bounded_field(issue, "title", max_title_len),
    url = require_bounded_field(issue, "url", max_key_len),
    updated_at = require_bounded_field(issue, "updated_at", max_key_len),
    source_ref = M.normalize_source_ref(require_field(issue, "source_ref")),
  }
end

function M.bounded_text(value, limit)
  local text = tostring(value or "")
  if #text <= limit then
    return text
  end
  return text:sub(1, limit)
end

function M.max_body_len()
  return max_body_len
end

-- Fail-closed gate before raising to consensus: the derived proposal must satisfy consensus's
-- own eligibility (path-safe bounded keys, bounded title/body, valid source_ref) AND its
-- proposal_id must round-trip so the reply department can recover repo/issue_number.
function M.validate_proposal(proposal)
  if type(proposal) ~= "table" then
    return false
  end
  if proposal.schema ~= "consensus.proposal.v1" then
    return false
  end
  if not is_path_safe_key(proposal.proposal_id, max_key_len) then
    return false
  end
  if not is_path_safe_key(proposal.dedup_key, max_key_len) then
    return false
  end
  local repo, issue_number = M.parse_proposal_id(proposal.proposal_id)
  if repo == nil or issue_number == nil then
    return false
  end
  if not is_bounded_string(proposal.title, max_title_len) then
    return false
  end
  if not is_bounded_string(proposal.body, max_body_len) then
    return false
  end
  return has_bounded_source_ref(proposal.source_ref)
end

-- Fail-closed gate before raising a reply: a malformed consensus_reached (missing/oversized
-- body or source_ref) must not produce an empty reply nor wrongly mark the issue replied.
function M.validate_reached(reached)
  if type(reached) ~= "table" then
    return false
  end
  if not is_bounded_string(reached.proposal_id, max_key_len) then
    return false
  end
  if not is_bounded_string(reached.body, max_body_len) then
    return false
  end
  return has_bounded_source_ref(reached.source_ref)
end

return M
