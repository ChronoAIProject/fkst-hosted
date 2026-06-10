local core = require("core")

local M = {}

M.spec = {
  consumes = { "github_pr_comment_request" },
  stall_window = "30s",
}

local function has_required_fields(payload)
  return payload.pr_number ~= nil and payload.body ~= nil and payload.dedup_key ~= nil
end

local function log_outbound(payload, repo, write_env)
  if repo == nil or repo == "" or not has_required_fields(payload) then
    return
  end

  local mode = write_env == "1" and "real" or "dry-run"
  local fields = {
    "mode=" .. mode,
    "repo=" .. tostring(repo),
    "pr=" .. tostring(payload.pr_number),
    "dedup_key=" .. tostring(payload.dedup_key),
  }
  if mode == "dry-run" then
    table.insert(fields, "reason=FKST_GITHUB_WRITE!=1")
  end
  core.log_line("info", "github_pr_comment", "OUTBOUND", fields)
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
    core.log_error_fact("error", "github_pr_comment", "FAILURE", core.error_class_from_message(err), "github_pr_comment_request", err, {
      source_ref = payload.source_ref,
      terminal = false,
    })
    error(err)
  end
end

function pipeline(event)
  local payload = event.payload or {}
  write_with_outbound_log(payload, {
    kind = "pr",
    number = payload.pr_number,
    number_field = "pr_number",
    view_comments_cmd = core.gh_pr_view_comments_cmd,
    comment_cmd = core.gh_pr_comment_cmd,
    view_label = "gh pr view",
    comment_label = "gh pr comment",
  })
end

return M
