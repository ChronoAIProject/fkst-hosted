local S = {}

function S.install(M)
local max_login_len = 80

local function issue_author_login(issue)
  if type(issue) ~= "table" then
    return nil
  end
  if issue.author_login ~= nil and tostring(issue.author_login) ~= "" then
    return tostring(issue.author_login)
  end
  if type(issue.author) == "table" and issue.author.login ~= nil and tostring(issue.author.login) ~= "" then
    return tostring(issue.author.login)
  end
  if type(issue.user) == "table" and issue.user.login ~= nil and tostring(issue.user.login) ~= "" then
    return tostring(issue.user.login)
  end
  return nil
end

local function safe_marker_attr(value)
  local text = tostring(value or ""):gsub("<!%-%- fkst:[^\n]*%-%->", " ")
  text = text:gsub("&lt;!%-%- fkst:[^\n]*%-%-&gt;", " ")
  text = text:gsub("%c", " "):gsub('"', "'"):gsub("[<>]", ""):gsub("%s+", " ")
  text = text:gsub("^%s+", ""):gsub("%s+$", "")
  if #text > 240 then
    text = M.truncate_utf8(text, 240)
  end
  return text
end

function M.issue_author_login(issue)
  return issue_author_login(issue)
end

function M.fork_issue_dedup_key(repo, issue_number)
  if not M.issue_ref_round_trips(repo, issue_number) then
    error("github-devloop: invalid fork issue target")
  end
  return M._dedup_key({
    "github-devloop",
    "fork",
    M.safe_repo(repo),
    "issue",
    M.safe_issue(issue_number),
    "v1",
  })
end

function M.has_trusted_issue_create_parent_marker(comments, dedup_key, bot_login)
  if type(comments) ~= "table" then
    return false
  end
  local create_pattern = "<!%-%- fkst:github%-proxy:issue%-create%-intent:v1.-%-%->"
  local created_pattern = "<!%-%- fkst:github%-proxy:issue%-created:v1.-%-%->"
  for _, comment in ipairs(comments) do
    if M.comment_author_login(comment) == tostring(bot_login) then
      local body = M.comment_body(comment)
      for marker in body:gmatch(create_pattern) do
        if marker:match('dedup="([^"]+)"') == tostring(dedup_key) then
          return true
        end
      end
      for marker in body:gmatch(created_pattern) do
        if marker:match('dedup="([^"]+)"') == tostring(dedup_key) then
          return true
        end
      end
    end
  end
  return false
end

function M.fork_issue_title(issue_number, original_title)
  local title = tostring(original_title or "Issue")
  title = title:gsub("%c", " "):gsub("%s+", " "):gsub("^%s+", ""):gsub("%s+$", "")
  if title == "" then
    title = "Issue"
  end
  local prefix = "Fork of #" .. tostring(issue_number) .. ": "
  local limit = M._max_title_len - #prefix
  if limit < 1 then
    limit = 1
  end
  if #title > limit then
    title = M.truncate_utf8(title, limit)
  end
  return prefix .. title
end

function M.fork_origin_marker(repo, issue_number, author_login, source_ref)
  local normalized = M.normalize_source_ref(source_ref or M.issue_source_ref(repo, issue_number))
  return '<!-- fkst:github-devloop:fork-origin:v1 repo="' .. safe_marker_attr(repo)
    .. '" issue="' .. safe_marker_attr(issue_number)
    .. '" author="' .. safe_marker_attr(author_login or "unknown")
    .. '" source_ref_kind="' .. safe_marker_attr(normalized.kind)
    .. '" source_ref="' .. safe_marker_attr(normalized.ref)
    .. '" -->'
end

local function fork_origin_fact_from_text(M, text)
  for marker in tostring(text or ""):gmatch("<!%-%- fkst:github%-devloop:fork%-origin:v1.-%-%->") do
    local source_ref = {
      kind = marker:match('source_ref_kind="([^"]+)"'),
      ref = marker:match('source_ref="([^"]+)"'),
    }
    local repo, issue_number = M.parse_issue_source_ref(source_ref)
    if repo ~= nil and issue_number ~= nil then
      return {
        repo = repo,
        issue_number = issue_number,
        source_ref = source_ref,
      }
    end
  end
  return nil
end

function M.fork_origin_fact(entity)
  if type(entity) ~= "table" then
    return nil
  end
  if M.issue_author_login(entity) == M.claim_owner() then
    local body_fact = fork_origin_fact_from_text(M, entity.body)
    if body_fact ~= nil then
      return body_fact
    end
  end
  for _, comment in ipairs(M._trusted_marker_comments(entity.comments)) do
    local comment_fact = fork_origin_fact_from_text(M, M.comment_body(comment))
    if comment_fact ~= nil then
      return comment_fact
    end
  end
  local title_issue = tostring(entity.title or ""):match("^Fork of #(%d+):")
  if title_issue ~= nil then
    local source_ref = M.issue_source_ref(entity.repo, title_issue)
    local repo, issue_number = M.parse_issue_source_ref(source_ref)
    if repo ~= nil and issue_number ~= nil then
      return {
        repo = repo,
        issue_number = issue_number,
        source_ref = source_ref,
      }
    end
  end
  return nil
end

function M.rederive_issue_state(repo, issue_number)
  local view = M.gh_exec({ cmd = M.gh_issue_view_state_cmd(repo, issue_number), timeout = 30 })
  if view.exit_code ~= 0 then
    error("github-devloop: gh issue source_ref state recheck failed: " .. tostring(view.stderr))
  end
  return M.parse_issue_view_state(view.stdout)
end

function M.rederive_issue_is_open(repo, issue_number)
  local current = M.rederive_issue_state(repo, issue_number)
  return tostring(current.state or ""):upper() == "OPEN", current
end

function M.fork_issue_body(repo, issue_number, author_login, source_ref)
  local normalized = M.normalize_source_ref(source_ref or M.issue_source_ref(repo, issue_number))
  return table.concat({
    "Self-owned fork for isolated implementation.",
    "",
    "Original: " .. tostring(repo) .. "#" .. tostring(issue_number),
    "Original author: " .. tostring(author_login or "unknown"),
    "Source ref: " .. tostring(normalized.kind) .. ":" .. tostring(normalized.ref),
    "",
    M.fork_origin_marker(repo, issue_number, author_login, normalized),
  }, "\n")
end

function M.build_fork_issue_create_request(repo, issue_number, current, source_ref)
  if tostring(current and current.state or ""):upper() ~= "OPEN" then
    return nil, "original-closed"
  end
  local author_login = issue_author_login(current)
  if author_login == nil or #author_login > max_login_len then
    return nil, "author-unknown"
  end
  local dedup_key = M.fork_issue_dedup_key(repo, issue_number)
  local normalized = M.normalize_source_ref(source_ref or M.issue_source_ref(repo, issue_number))
  return {
    schema = "github-proxy.issue-create.v1",
    repo = tostring(repo),
    title = M.fork_issue_title(issue_number, current and current.title),
    body = M.fork_issue_body(repo, issue_number, author_login, normalized),
    assignees = { M.claim_owner() },
    dedup_key = dedup_key,
    external_effect_saga = "fork-and-block",
    external_effect_step = "create-fork",
    parent_comment_target = {
      repo = tostring(repo),
      issue_number = tonumber(issue_number),
    },
    post_create_blocked_by = {
      blocked_issue_number = tonumber(issue_number),
      dedup_key = dedup_key .. "/blocked-by",
      external_effect_saga = "fork-and-block",
      external_effect_step = "block-original",
    },
    source_ref = normalized,
  }, nil
end

end

return S
