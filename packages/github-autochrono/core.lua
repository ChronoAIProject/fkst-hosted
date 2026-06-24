local M = {}
local payload_validator = require("contract.payload")

local require_field = payload_validator.require_field
local error_context = "github-autochrono glue"


local function require_source_ref(payload)
  local source_ref = require_field(payload, "source_ref", error_context)
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
    repo = require_field(payload, "repo", error_context),
    issue_number = require_field(payload, "number", error_context),
    title = require_field(payload, "title", error_context),
    url = require_field(payload, "url", error_context),
    state = require_field(payload, "state", error_context),
    updated_at = require_field(payload, "updated_at", error_context),
    source_ref = require_source_ref(payload),
    dedup_key = require_field(payload, "dedup_key", error_context),
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
    repo = require_field(payload, "repo", error_context),
    issue_number = require_field(payload, "issue_number", error_context),
    body = require_field(payload, "body", error_context),
    dedup_key = require_field(payload, "dedup_key", error_context),
    source_ref = require_source_ref(payload),
  }
end

return M
