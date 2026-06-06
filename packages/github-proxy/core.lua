local M = {}

local allowed_env = {
  FKST_GITHUB_REPO = true,
  FKST_GITHUB_BOT_LOGIN = true,
  FKST_GITHUB_WRITE = true,
}
local trusted_bot_login = nil

local function shell_single_quote(value)
  return "'" .. tostring(value):gsub("'", "'\\''") .. "'"
end

function M.read_env_command(name)
  if not allowed_env[name] then
    error("env name is not allowed: " .. tostring(name))
  end
  return 'printf %s "$' .. name .. '"'
end

function M.read_env(name, exec)
  local run = exec or exec_sync
  if type(run) ~= "function" then
    error("read_env requires exec_sync")
  end
  local out = run(M.read_env_command(name))
  if out.exit_code ~= 0 then
    return nil
  end
  if out.stdout == "" then
    return nil
  end
  return out.stdout
end

function M.configure_trusted_bot_login(login)
  if login == nil or tostring(login) == "" then
    trusted_bot_login = nil
    return nil
  end
  trusted_bot_login = tostring(login)
  return trusted_bot_login
end

function M.assert_trusted_bot_configured()
  local login = M.read_env("FKST_GITHUB_BOT_LOGIN")
  if login ~= nil then
    M.configure_trusted_bot_login(login)
  end

  if trusted_bot_login == nil then
    error("github-proxy: FKST_GITHUB_BOT_LOGIN is required when FKST_GITHUB_WRITE=1")
  end
  return trusted_bot_login
end

function M.entity_cache_key(repo, entity_type, number)
  return "github-proxy/" .. tostring(entity_type) .. "/" .. tostring(repo) .. "/" .. tostring(number)
end

function M.entity_dedup_key(repo, entity_type, number, updated_at)
  return tostring(repo)
    .. "#"
    .. tostring(entity_type)
    .. "#"
    .. tostring(number)
    .. "@"
    .. tostring(updated_at)
end

function M.issue_dedup_key(repo, number, updated_at)
  return M.entity_dedup_key(repo, "issue", number, updated_at)
end

-- Stable source pointer for the durable-delivery engine: a reliable consumer
-- re-derives the current entity from this ref (e.g. `gh issue view`) instead of
-- trusting a possibly-stale payload. ref is the entity identity WITHOUT the
-- version (updated_at lives in dedup_key / the payload).
function M.entity_source_ref(repo, entity_type, number)
  return {
    kind = "external",
    ref = tostring(repo) .. "#" .. tostring(entity_type) .. "/" .. tostring(number),
  }
end

function M.comment_marker(dedup_key)
  return "<!-- fkst:github-proxy:comment:" .. tostring(dedup_key) .. " -->"
end

function M.has_marker(comments_text, dedup_key)
  if comments_text == nil or comments_text == "" then
    return false
  end
  return tostring(comments_text):find(M.comment_marker(dedup_key), 1, true) ~= nil
end

local function comment_body(comment)
  if type(comment) == "table" then
    return tostring(comment.body or "")
  end
  return tostring(comment or "")
end

local function comment_author_login(comment)
  if type(comment) == "table" then
    if comment.author_login ~= nil then
      return tostring(comment.author_login)
    end
    if type(comment.author) == "table" and comment.author.login ~= nil then
      return tostring(comment.author.login)
    end
  end
  return nil
end

function M.parse_issue_comments(gh_json_stdout)
  local decoded = json.decode(gh_json_stdout or "{}")
  local comments = {}
  for _, comment in ipairs(decoded.comments or {}) do
    table.insert(comments, {
      body = comment_body(comment),
      author_login = comment_author_login(comment),
    })
  end
  return comments
end

function M.has_trusted_marker(comments, dedup_key, bot_login)
  if type(comments) ~= "table" then
    return false
  end
  local marker = M.comment_marker(dedup_key)
  for _, comment in ipairs(comments) do
    if comment_author_login(comment) == bot_login and comment_body(comment):find(marker, 1, true) ~= nil then
      return true
    end
  end
  return false
end

-- Decodes gh --json output via the engine-provided json.decode; requires a json-capable substrate runtime.
function M.parse_entity_list(gh_json_stdout)
  local decoded = json.decode(gh_json_stdout or "[]")
  local entities = {}
  for _, item in ipairs(decoded) do
    local labels = {}
    for _, label in ipairs(item.labels or {}) do
      if type(label) == "table" and label.name ~= nil then
        table.insert(labels, tostring(label.name))
      elseif type(label) == "string" then
        table.insert(labels, label)
      end
    end
    table.insert(entities, {
      number = item.number,
      title = item.title,
      url = item.url,
      updated_at = item.updatedAt or item.updated_at,
      state = item.state,
      labels = labels,
    })
  end
  return entities
end

function M.parse_issue_list(gh_json_stdout)
  return M.parse_entity_list(gh_json_stdout)
end

function M.gh_issue_list_cmd(repo)
  return "gh issue list --repo " .. shell_single_quote(repo) .. " --state all --json number,title,updatedAt,url,state,labels"
end

function M.gh_pr_list_cmd(repo)
  return "gh pr list --repo " .. shell_single_quote(repo) .. " --state all --json number,title,updatedAt,url,state,labels"
end

function M.gh_issue_view_comments_cmd(repo, issue_number)
  return "gh issue view " .. shell_single_quote(issue_number)
    .. " --repo " .. shell_single_quote(repo)
    .. " --json comments"
end

function M.gh_issue_view_labels_cmd(repo, issue_number)
  return "gh issue view " .. shell_single_quote(issue_number)
    .. " --repo " .. shell_single_quote(repo)
    .. " --json labels"
end

function M.parse_issue_labels(gh_json_stdout)
  local decoded = json.decode(gh_json_stdout or "{}")
  local labels = {}
  for _, label in ipairs(decoded.labels or {}) do
    if type(label) == "table" and label.name ~= nil then
      table.insert(labels, tostring(label.name))
    elseif type(label) == "string" then
      table.insert(labels, label)
    end
  end
  return labels
end

function M.gh_issue_comment_cmd(repo, issue_number, body_file)
  return "gh issue comment " .. shell_single_quote(issue_number)
    .. " --repo " .. shell_single_quote(repo)
    .. " --body-file " .. shell_single_quote(body_file)
end

function M.gh_issue_edit_labels_cmd(repo, issue_number, add_labels, remove_labels)
  local cmd = "gh issue edit " .. shell_single_quote(issue_number)
    .. " --repo " .. shell_single_quote(repo)
  for _, label in ipairs(add_labels or {}) do
    cmd = cmd .. " --add-label " .. shell_single_quote(label)
  end
  for _, label in ipairs(remove_labels or {}) do
    cmd = cmd .. " --remove-label " .. shell_single_quote(label)
  end
  return cmd
end

return M
