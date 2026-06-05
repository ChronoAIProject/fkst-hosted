local M = {}

local function require_field(payload, name)
  local value = payload[name]
  if value == nil or value == "" then
    error("github-autochrono glue: missing " .. name)
  end
  return value
end

local function require_source_ref(payload)
  local source_ref = require_field(payload, "source_ref")
  if type(source_ref) ~= "table" or source_ref.kind == nil or source_ref.ref == nil then
    error("github-autochrono glue: invalid source_ref")
  end
  return source_ref
end

function M.entity_to_issue(payload)
  if type(payload) ~= "table" then
    error("github-autochrono glue: payload must be a table")
  end
  if payload.schema ~= "github-proxy.v1" then
    error("github-autochrono glue: unsupported entity schema")
  end
  if payload.type ~= "issue" then
    error("github-autochrono glue: entity is not an issue")
  end

  return {
    schema = "autochrono.issue.v1",
    repo = require_field(payload, "repo"),
    issue_number = require_field(payload, "number"),
    title = require_field(payload, "title"),
    url = require_field(payload, "url"),
    state = require_field(payload, "state"),
    updated_at = require_field(payload, "updated_at"),
    source_ref = require_source_ref(payload),
    dedup_key = require_field(payload, "dedup_key"),
  }
end

function M.reply_to_comment_request(payload)
  if type(payload) ~= "table" then
    error("github-autochrono glue: payload must be a table")
  end
  if payload.schema ~= "autochrono.reply.v1" then
    error("github-autochrono glue: unsupported reply schema")
  end

  return {
    schema = "github-proxy.v1",
    repo = require_field(payload, "repo"),
    issue_number = require_field(payload, "issue_number"),
    body = require_field(payload, "body"),
    dedup_key = require_field(payload, "dedup_key"),
    source_ref = require_source_ref(payload),
  }
end

return M
