local M = {}
local payload_validator = require("std.payload")
local source_refs = require("std.source_ref")
local strings = require("std.strings")

function M.persistence_class()
  return "judgment_pipeline"
end

local max_key_len = 200
local max_title_len = 240
local max_body_len = 12000
local max_repo_key_len = 100
local max_issue_key_len = 30
local max_update_key_len = 50

local is_bounded_string = strings.is_bounded_string

-- Mirrors consensus's is_path_safe_key so propose can fail closed BEFORE raising a
-- proposal the consensus engine would reject (and before wrongly writing its cache).
local is_path_safe_key = strings.is_path_safe_key

local function require_bounded_field(payload, name, limit)
  local value = payload_validator.require_field(payload, name, "autochrono")
  if not is_bounded_string(tostring(value), limit) then
    error("autochrono: invalid " .. name)
  end
  return value
end

local function safe_repo(repo)
  return strings.sanitize_key(repo, max_key_len):sub(1, max_repo_key_len):gsub("/+$", "")
end

local function safe_issue_number(issue_number)
  return strings.sanitize_key(issue_number, max_key_len):sub(1, max_issue_key_len):gsub("/+$", "")
end

local function safe_updated_at(updated_at)
  return strings.sanitize_key(updated_at, max_key_len):sub(1, max_update_key_len):gsub("/+$", "")
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

function M.issue_ref_round_trips(repo, issue_number)
  local repo_text = tostring(repo)
  local issue_number_text = tostring(issue_number)

  if safe_repo(repo) ~= repo_text then
    return false
  end
  if safe_issue_number(issue_number) ~= issue_number_text then
    return false
  end

  local parsed_repo, parsed_issue_number = M.parse_proposal_id(M.proposal_id(repo, issue_number))
  return parsed_repo == repo_text and parsed_issue_number == issue_number_text
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
  if not M.issue_ref_round_trips(issue.repo, issue.issue_number) then
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
    source_ref = M.normalize_source_ref(payload_validator.require_field(issue, "source_ref", "autochrono")),
  }
end

function M.render_template(template, vars)
  if type(template) ~= "string" then
    error("autochrono: template must be a string")
  end
  if type(vars) ~= "table" then
    error("autochrono: template vars must be a table")
  end

  return (template:gsub("{{([%w_]+)}}", function(name)
    local value = vars[name]
    if value == nil then
      error("autochrono: missing template var " .. name)
    end
    return tostring(value)
  end))
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
  if proposal.proposal_id ~= M.proposal_id(repo, issue_number) then
    return false
  end
  if not is_bounded_string(proposal.title, max_title_len) then
    return false
  end
  if not is_bounded_string(proposal.body, max_body_len) then
    return false
  end
  return source_refs.has_bounded_source_ref(proposal.source_ref, max_key_len)
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
  return source_refs.has_bounded_source_ref(reached.source_ref, max_key_len)
end

return M
