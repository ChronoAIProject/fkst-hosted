local core = require("core")

local M = {}

M.spec = {
  consumes = { "github_issue_comment_request" },
  stall_window = "30s",
}

local function has_required_fields(payload)
  return payload.issue_number ~= nil and payload.body ~= nil and payload.dedup_key ~= nil
end

local function log_outbound(payload, repo, write_env)
  if repo == nil or repo == "" or not has_required_fields(payload) then
    return
  end

  local mode = write_env == "1" and "real" or "dry-run"
  local fields = {
    "mode=" .. mode,
    "repo=" .. tostring(repo),
    "issue=" .. tostring(payload.issue_number),
    "dedup_key=" .. tostring(payload.dedup_key),
  }
  if mode == "dry-run" then
    table.insert(fields, "reason=FKST_GITHUB_WRITE!=1")
  end
  core.log_line("info", "github_comment", "OUTBOUND", fields)
end

local function write_with_outbound_log(payload, target)
  local repo = payload.repo
  local logged = false
  local read_env = core.read_env
  core.read_env = function(name, exec)
    local value = read_env(name, exec)
    if name == "FKST_GITHUB_REPO" and (repo == nil or repo == "") then
      repo = value
    end
    if name == "FKST_GITHUB_WRITE" and not logged then
      log_outbound(payload, repo, value)
      logged = true
    end
    return value
  end

  local ok, err = pcall(function()
    core.write_comment_request(payload, target)
  end)
  core.read_env = read_env
  if not ok then
    error(err)
  end
end

function pipeline(event)
  local payload = event.payload or {}
  write_with_outbound_log(payload, {
    kind = "issue",
    number = payload.issue_number,
    number_field = "issue_number",
    view_comments_cmd = core.gh_issue_view_comments_cmd,
    comment_cmd = core.gh_issue_comment_cmd,
    view_label = "gh issue view",
    comment_label = "gh issue comment",
  })
end

pipeline = core.wrap_pipeline_failure("github_comment", pipeline)

return M
