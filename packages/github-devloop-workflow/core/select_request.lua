local fail = require("core.errors").fail
local workflow_marker = require("core.marker")

local devloop_base = require("devloop.base")
local base_ids = require("devloop.base_ids")
local claims = require("devloop.claims")
local devloop_marker_builders = require("devloop.markers.builders")
local source_refs = require("contract.source_ref")
local strings = require("contract.strings")

local M = {}

local function require_string(value, path)
  if type(value) ~= "string" then
    return false, fail(path, "not_string", "must be a string")
  end
  if value == "" then
    return false, fail(path, "empty", "must not be empty")
  end
  return true, nil
end

local function require_value(value, path)
  if value == nil or tostring(value) == "" then
    return false, fail(path, "empty", "must not be empty")
  end
  return true, nil
end

local function validate_module(root)
  if type(root) ~= "table" then
    return false, fail("M", "not_object", "must be a module table")
  end
  if type(root._max_meta_reason_len) ~= "number" or root._max_meta_reason_len < 1 then
    return false, fail("M._max_meta_reason_len", "invalid_bound", "must be a positive number")
  end
  if type(root._max_dedup_len) ~= "number" or root._max_dedup_len < 1 then
    return false, fail("M._max_dedup_len", "invalid_bound", "must be a positive number")
  end
  return true, nil
end

local function validate_candidate(root, candidate)
  if type(candidate) ~= "table" then
    return false, fail("candidate", "not_object", "must be an object")
  end
  local ok, err = require_string(candidate.proposal_id, "candidate.proposal_id")
  if not ok then return false, err end
  ok, err = require_string(candidate.dedup_key, "candidate.dedup_key")
  if not ok then return false, err end
  if not strings.is_bounded_string(candidate.dedup_key, root._max_dedup_len) then
    return false, fail("candidate.dedup_key", "invalid_dedup_key", "must be a bounded string")
  end
  if not source_refs.has_bounded_source_ref(candidate.source_ref, base_ids.max_key_len) then
    return false, fail("candidate.source_ref", "invalid_source_ref", "must be a bounded source_ref")
  end
  return true, nil
end

local function bounded_reason(root, reason)
  local safe_reason = devloop_base.neutralize_untrusted_comment_text(reason or "")
  if safe_reason == "" then
    safe_reason = "(no reason provided)"
  end
  if #safe_reason > root._max_meta_reason_len then
    safe_reason = base_ids.truncate_utf8(safe_reason, root._max_meta_reason_len)
  end
  return safe_reason
end

function M.build_blueprint_decision_comment_request(root, repo, issue_number, candidate, blueprint_id, plan_digest, reason)
  local ok, err = validate_module(root)
  if not ok then return nil, err end
  ok, err = require_string(repo, "repo")
  if not ok then return nil, err end
  ok, err = require_value(issue_number, "issue_number")
  if not ok then return nil, err end
  ok, err = validate_candidate(root, candidate)
  if not ok then return nil, err end

  local blueprint_marker, marker_err = workflow_marker.build_blueprint_marker(candidate.proposal_id, blueprint_id, plan_digest)
  if blueprint_marker == nil then
    return nil, marker_err
  end

  -- Workflow origins are tracking umbrellas, not executable devloop work items.
  local track_marker = devloop_marker_builders.intake_decision_marker(
    candidate.proposal_id,
    "track",
    candidate.dedup_key,
    "standard"
  )
  local safe_reason = bounded_reason(root, reason)
  local payload = {
    schema = "github-proxy.v1",
    repo = repo,
    issue_number = issue_number,
    body = "This issue is managed by workflow " .. tostring(blueprint_id) .. "."
      .. "\n\nReason:\n" .. safe_reason
      .. "\n\n" .. blueprint_marker
      .. "\n" .. track_marker,
    dedup_key = base_ids.dedup_key({
      "workflow",
      "blueprint-decision",
      tostring(candidate.proposal_id),
      tostring(candidate.dedup_key),
    }),
    source_ref = base_ids.normalize_source_ref(candidate.source_ref),
  }
  return claims.attach_issue_claim(payload, candidate.source_ref), nil
end

function M.install(target)
  target.build_blueprint_decision_comment_request = function(...)
    return M.build_blueprint_decision_comment_request(target, ...)
  end
end

return M
