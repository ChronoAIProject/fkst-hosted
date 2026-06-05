local core = require("core")

local M = {}

M.spec = {
  consumes = { "github_issue_comment_request" },
  stall_window = "30s",
}

local MAX_RUNTIME_ID_LEN = 180

local function safe_segment(value)
  local safe = tostring(value or ""):gsub("[^%w._-]", "_")
  safe = safe:gsub("_+", "_"):gsub("^_+", ""):gsub("_+$", "")
  if safe == "" then
    return "empty"
  end
  return safe
end

local function runtime_identity(repo, issue_number)
  local id = "comment-" .. safe_segment(repo) .. "-issue-" .. safe_segment(issue_number)
  if #id > MAX_RUNTIME_ID_LEN then
    return id:sub(1, MAX_RUNTIME_ID_LEN)
  end
  return id
end

local function temp_body_file(repo, issue_number)
  return "/tmp/fkst-github-proxy-" .. runtime_identity(repo, issue_number) .. ".md"
end

local function lock_name(repo, issue_number)
  return "github-proxy/" .. runtime_identity(repo, issue_number)
end

function pipeline(event)
  local payload = event.payload or {}
  local repo = payload.repo
  if repo == nil or repo == "" then
    repo = core.read_env("FKST_GITHUB_REPO")
  end
  if repo == nil or repo == "" then
    log.warn("github-proxy: comment request missing repo")
    return
  end
  if payload.issue_number == nil or payload.body == nil or payload.dedup_key == nil then
    log.warn("github-proxy: comment request missing issue_number, body, or dedup_key")
    return
  end

  if core.read_env("FKST_GITHUB_WRITE") ~= "1" then
    log.info("github-proxy dry-run: would comment on " .. repo .. "#" .. tostring(payload.issue_number))
    return
  end

  with_lock(lock_name(repo, payload.issue_number), function()
    local view = exec_sync({ cmd = core.gh_issue_view_comments_cmd(repo, payload.issue_number), timeout = 30 })
    if view.exit_code ~= 0 then
      -- A re-derive command failure must NOT silent-ack a reliable comment request; error so
      -- delivery retries (otherwise a result marker comment could be permanently lost).
      error("github-proxy: gh issue view failed: " .. tostring(view.stderr))
    end
    if core.has_marker(view.stdout, payload.dedup_key) then
      log.info("github-proxy: comment marker already present")
      return
    end

    local body = tostring(payload.body) .. "\n\n" .. core.comment_marker(payload.dedup_key) .. "\n"
    local path = temp_body_file(repo, payload.issue_number)
    file.write(path, body)
    local comment = exec_sync({ cmd = core.gh_issue_comment_cmd(repo, payload.issue_number, path), timeout = 30 })
    if comment.exit_code ~= 0 then
      error("github-proxy: gh issue comment failed: " .. tostring(comment.stderr))
    end
  end)
end

return M
