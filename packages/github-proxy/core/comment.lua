local S = {}

function S.install(M)
local max_runtime_id_len = 180

local function shell_single_quote(value)
  return "'" .. tostring(value):gsub("'", "'\\''") .. "'"
end

local function safe_runtime_segment(value)
  local safe = tostring(value or ""):gsub("[^%w._-]", "_")
  safe = safe:gsub("_+", "_"):gsub("^_+", ""):gsub("_+$", "")
  return safe == "" and "empty" or safe
end

local function comment_runtime_identity(repo, kind, number)
  local id = "comment-" .. safe_runtime_segment(repo)
    .. "-" .. safe_runtime_segment(kind)
    .. "-" .. safe_runtime_segment(number)
  if #id > max_runtime_id_len then
    return id:sub(1, max_runtime_id_len)
  end
  return id
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

function M._comment_body(comment)
  return comment_body(comment)
end

function M._comment_author_login(comment)
  return comment_author_login(comment)
end

local function comment_id(comment)
  if type(comment) ~= "table" then
    return nil
  end
  local id = comment.id or comment.databaseId or comment.database_id
  if id == nil or tostring(id) == "" then
    return nil
  end
  return tostring(id)
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

function M.parse_issue_comments(gh_json_stdout)
  local decoded = json.decode(gh_json_stdout or "{}")
  local comments = {}
  for _, comment in ipairs(decoded.comments or {}) do
    table.insert(comments, {
      id = comment_id(comment),
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

function M.has_trusted_comment_fragment(comments, fragment, bot_login)
  if type(comments) ~= "table" or type(fragment) ~= "string" or fragment == "" then
    return false
  end
  for _, comment in ipairs(comments) do
    if comment_author_login(comment) == bot_login and comment_body(comment):find(fragment, 1, true) ~= nil then
      return true
    end
  end
  return false
end

function M.trusted_comment_with_fragment(comments, fragment, bot_login)
  if type(comments) ~= "table" or type(fragment) ~= "string" or fragment == "" then
    return nil
  end
  for _, comment in ipairs(comments) do
    if comment_author_login(comment) == bot_login and comment_body(comment):find(fragment, 1, true) ~= nil then
      return comment
    end
  end
  return nil
end

function M.gh_pr_comment_cmd(repo, pr_number, body_file)
  return "gh pr comment " .. shell_single_quote(pr_number)
    .. " --repo " .. shell_single_quote(repo)
    .. " --body-file " .. shell_single_quote(body_file)
end

function M.gh_pr_view_comments_cmd(repo, pr_number)
  return "gh pr view " .. shell_single_quote(pr_number)
    .. " --repo " .. shell_single_quote(repo)
    .. " --json comments"
end

function M.gh_issue_view_comments_cmd(repo, issue_number)
  return "gh issue view " .. shell_single_quote(issue_number)
    .. " --repo " .. shell_single_quote(repo)
    .. " --json comments"
end

function M.gh_issue_comment_cmd(repo, issue_number, body_file)
  return "gh issue comment " .. shell_single_quote(issue_number)
    .. " --repo " .. shell_single_quote(repo)
    .. " --body-file " .. shell_single_quote(body_file)
end

function M.gh_comment_edit_cmd(repo, comment_id_value, body_file)
  if comment_id_value == nil or tostring(comment_id_value) == "" then
    error("github-proxy: invalid comment id")
  end
  return "gh api --method PATCH "
    .. shell_single_quote("repos/" .. tostring(repo) .. "/issues/comments/" .. tostring(comment_id_value))
    .. " --field body@" .. shell_single_quote(body_file)
end

function M.write_comment_request(payload, target)
  local repo = payload.repo
  if repo == nil or repo == "" then
    repo = M.read_env("FKST_GITHUB_REPO")
  end
  if repo == nil or repo == "" then
    log.warn("github-proxy: comment request missing repo")
    return
  end
  if target.number == nil or payload.body == nil or payload.dedup_key == nil then
    log.warn("github-proxy: comment request missing " .. tostring(target.number_field) .. ", body, or dedup_key")
    return
  end

  if M.read_env("FKST_GITHUB_WRITE") ~= "1" then
    log.info("github-proxy dry-run: would comment on " .. repo .. "#" .. tostring(target.number))
    return
  end
  local bot_login = M.assert_trusted_bot_configured()

  local runtime_id = comment_runtime_identity(repo, target.kind, target.number)
  with_lock("github-proxy/" .. runtime_id, function()
    local view = M.gh_exec(target.view_comments_cmd(repo, target.number), 30, target.view_label)
    local comments = M.parse_issue_comments(view.stdout)
    local replace_marker = payload.replace_marker
    local existing = nil
    if replace_marker ~= nil and tostring(replace_marker) ~= "" then
      existing = M.trusted_comment_with_fragment(comments, tostring(replace_marker), bot_login)
    elseif M.has_trusted_marker(comments, payload.dedup_key, bot_login) then
      log.info("github-proxy: comment marker already present")
      return
    end

    local body = tostring(payload.body) .. "\n\n" .. M.comment_marker(payload.dedup_key) .. "\n"
    local path = "/tmp/fkst-github-proxy-" .. runtime_id .. ".md"
    file.write(path, body)
    if existing ~= nil and existing.id ~= nil then
      M.gh_exec(M.gh_comment_edit_cmd(repo, existing.id, path), 30, "gh comment edit")
      return
    end
    if existing ~= nil then
      log.warn("github-proxy: replace marker comment missing id; creating a fresh comment")
    end
    M.gh_exec(target.comment_cmd(repo, target.number, path), 30, target.comment_label)
  end)
end

end

return S
