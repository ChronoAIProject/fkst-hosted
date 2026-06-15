local S = {}

function S.install(M)
local max_title_len = 240
local max_body_len = 12000
local max_label_len = 80
local max_login_len = 80
local max_dedup_len = 512
local max_runtime_id_len = 180
local max_issue_number_len = 32

local function shell_single_quote(value)
  return "'" .. tostring(value):gsub("'", "'\\''") .. "'"
end

local function is_bounded_string(value, limit)
  return type(value) == "string" and value ~= "" and #value <= limit
end

local function is_bounded_marker_value(value, limit)
  return is_bounded_string(value, limit)
    and tostring(value):find('[<>"\r\n]') == nil
end

local function is_positive_integer(value)
  local n = tonumber(value)
  return n ~= nil and n >= 1 and n % 1 == 0 and n <= 2147483647
end

local function safe_runtime_segment(value)
  local safe = tostring(value or ""):gsub("[^%w._-]", "_")
  safe = safe:gsub("_+", "_"):gsub("^_+", ""):gsub("_+$", "")
  if safe == "" then
    return "empty"
  end
  return safe
end

local function issue_create_runtime_identity(dedup_key)
  local id = "issue-create-" .. safe_runtime_segment(dedup_key)
  if #id > max_runtime_id_len then
    return id:sub(1, max_runtime_id_len)
  end
  return id
end

local function labels_arg(labels)
  local args = ""
  if type(labels) ~= "table" then
    return args
  end
  for _, label in ipairs(labels) do
    if is_bounded_string(label, max_label_len) then
      args = args .. " --label " .. shell_single_quote(label)
    end
  end
  return args
end

local function is_valid_login(value)
  return is_bounded_string(value, max_login_len)
    and tostring(value):find("^[%w%-%[%]_.]+$") ~= nil
end

local function assignees_arg(assignees)
  local args = ""
  if type(assignees) ~= "table" then
    return args
  end
  for _, login in ipairs(assignees) do
    if is_valid_login(login) then
      args = args .. " --assignee " .. shell_single_quote(login)
    end
  end
  return args
end

local function issue_author_login(issue)
  if type(issue) ~= "table" then
    return nil
  end
  if issue.author_login ~= nil then
    return tostring(issue.author_login)
  end
  if type(issue.author) == "table" and issue.author.login ~= nil then
    return tostring(issue.author.login)
  end
  return nil
end

function M.issue_create_marker(dedup_key)
  if not is_bounded_marker_value(dedup_key, max_dedup_len) then
    error("github-proxy: invalid issue-create dedup_key")
  end
  return "<!-- fkst:github-proxy:issue-create:" .. tostring(dedup_key) .. " -->"
end

function M.issue_created_marker(dedup_key, issue_number)
  if not is_bounded_marker_value(dedup_key, max_dedup_len) then
    error("github-proxy: invalid issue-created dedup_key")
  end
  local issue = tostring(issue_number or "unknown")
  if #issue > max_issue_number_len then
    issue = issue:sub(1, max_issue_number_len)
  end
  return '<!-- fkst:github-proxy:issue-created:v1 dedup="' .. tostring(dedup_key)
    .. '" issue="' .. issue
    .. '" -->'
end

function M.issue_create_intent_marker(dedup_key)
  if not is_bounded_marker_value(dedup_key, max_dedup_len) then
    error("github-proxy: invalid issue-create intent dedup_key")
  end
  return '<!-- fkst:github-proxy:issue-create-intent:v1 dedup="' .. tostring(dedup_key)
    .. '" -->'
end

function M.issue_create_lock_key(dedup_key)
  return "github-proxy/issue-create/" .. issue_create_runtime_identity(dedup_key)
end

function M.issue_create_once_key(dedup_key)
  return "github-proxy/issue-create-once/" .. issue_create_runtime_identity(dedup_key)
end

function M.gh_issue_create_search_cmd(repo, dedup_key)
  return "gh issue list --repo " .. shell_single_quote(repo)
    .. " --state all --limit 100"
    .. " --search " .. shell_single_quote(M.issue_create_marker(dedup_key))
    .. " --json number,title,state,author,body,url"
end

function M.gh_issue_create_cmd(repo, title, body_file, labels, assignees)
  return "gh issue create --repo " .. shell_single_quote(repo)
    .. " --title " .. shell_single_quote(title)
    .. " --body-file " .. shell_single_quote(body_file)
    .. labels_arg(labels)
    .. assignees_arg(assignees)
end

local function normalize_parent_comment_target(target)
  if target == nil then
    return nil
  end
  if type(target) ~= "table" or not is_bounded_string(target.repo, 200) then
    return false
  end
  if is_positive_integer(target.pr_number) then
    return {
      kind = "pr",
      repo = tostring(target.repo),
      number = tostring(target.pr_number),
    }
  end
  if is_positive_integer(target.issue_number) then
    return {
      kind = "issue",
      repo = tostring(target.repo),
      number = tostring(target.issue_number),
    }
  end
  return false
end

function M.gh_issue_create_parent_view_cmd(parent)
  if parent.kind == "pr" then
    return M.gh_pr_view_comments_cmd(parent.repo, parent.number)
  end
  return M.gh_issue_view_comments_cmd(parent.repo, parent.number)
end

function M.gh_issue_create_parent_comment_cmd(parent, body_file)
  if parent.kind == "pr" then
    return M.gh_pr_comment_cmd(parent.repo, parent.number, body_file)
  end
  return M.gh_issue_comment_cmd(parent.repo, parent.number, body_file)
end

function M.parse_issue_create_search(stdout)
  local decoded = json.decode(stdout or "[]")
  local issues = {}
  if type(decoded) ~= "table" then
    return issues
  end
  for _, issue in ipairs(decoded) do
    if type(issue) == "table" then
      table.insert(issues, {
        number = issue.number,
        title = issue.title,
        state = issue.state,
        body = tostring(issue.body or ""),
        author_login = issue_author_login(issue),
        url = issue.url,
      })
    end
  end
  return issues
end

function M.has_trusted_issue_create_marker(issues, dedup_key, bot_login)
  if type(issues) ~= "table" then
    return false
  end
  local marker = M.issue_create_marker(dedup_key)
  for _, issue in ipairs(issues) do
    if issue_author_login(issue) == tostring(bot_login)
      and tostring(issue.body or ""):find(marker, 1, true) ~= nil then
      return true
    end
  end
  return false
end

function M.has_trusted_issue_created_marker(comments, dedup_key, bot_login)
  if type(comments) ~= "table" then
    return false
  end
  local marker_pattern = "<!%-%- fkst:github%-proxy:issue%-created:v1.-%-%->"
  for _, comment in ipairs(comments) do
    if issue_author_login(comment) == tostring(bot_login) then
      local body = tostring(comment.body or "")
      for marker in body:gmatch(marker_pattern) do
        if marker:match('dedup="([^"]+)"') == tostring(dedup_key) then
          return true
        end
      end
    end
  end
  return false
end

function M.trusted_issue_created_number(comments, dedup_key, bot_login)
  if type(comments) ~= "table" then
    return nil
  end
  local marker_pattern = "<!%-%- fkst:github%-proxy:issue%-created:v1.-%-%->"
  for _, comment in ipairs(comments) do
    if issue_author_login(comment) == tostring(bot_login) then
      local body = tostring(comment.body or "")
      for marker in body:gmatch(marker_pattern) do
        if marker:match('dedup="([^"]+)"') == tostring(dedup_key) then
          local issue_number = marker:match('issue="(%d+)"')
          if is_positive_integer(issue_number) then
            return tostring(math.floor(tonumber(issue_number)))
          end
        end
      end
    end
  end
  return nil
end

function M.has_trusted_issue_create_intent_marker(comments, dedup_key, bot_login)
  if type(comments) ~= "table" then
    return false
  end
  local marker_pattern = "<!%-%- fkst:github%-proxy:issue%-create%-intent:v1.-%-%->"
  for _, comment in ipairs(comments) do
    if issue_author_login(comment) == tostring(bot_login) then
      local body = tostring(comment.body or "")
      for marker in body:gmatch(marker_pattern) do
        if marker:match('dedup="([^"]+)"') == tostring(dedup_key) then
          return true
        end
      end
    end
  end
  return false
end

function M.has_trusted_issue_create_parent_marker(comments, dedup_key, bot_login)
  return M.has_trusted_issue_create_intent_marker(comments, dedup_key, bot_login)
    or M.has_trusted_issue_created_marker(comments, dedup_key, bot_login)
end

function M.parse_created_issue_number(stdout)
  local text = tostring(stdout or "")
  local number = text:match("/issues/(%d+)")
  if number ~= nil then
    return number
  end
  return text:match("#(%d+)")
end

local function issue_created_marker_body_file(dedup_key)
  return "/tmp/fkst-github-proxy-created-" .. issue_create_runtime_identity(dedup_key) .. ".md"
end

local function issue_create_intent_marker_body_file(dedup_key)
  return "/tmp/fkst-github-proxy-intent-" .. issue_create_runtime_identity(dedup_key) .. ".md"
end

function M.validate_issue_create_payload(payload)
  if type(payload) ~= "table" then
    return false
  end
  if payload.schema ~= "github-proxy.issue-create.v1" then
    return false
  end
  if not is_bounded_string(payload.repo, 200) then
    return false
  end
  if not is_bounded_string(payload.title, max_title_len) then
    return false
  end
  if not is_bounded_string(payload.body, max_body_len) then
    return false
  end
  if not is_bounded_marker_value(payload.dedup_key, max_dedup_len) then
    return false
  end
  if type(payload.source_ref) ~= "table"
    or not is_bounded_string(payload.source_ref.kind, 80)
    or not is_bounded_string(payload.source_ref.ref, 200) then
    return false
  end
  if payload.labels ~= nil then
    if type(payload.labels) ~= "table" then
      return false
    end
    for _, label in ipairs(payload.labels) do
      if not is_bounded_string(label, max_label_len) then
        return false
      end
    end
  end
  if payload.assignees ~= nil then
    if type(payload.assignees) ~= "table" then
      return false
    end
    for _, login in ipairs(payload.assignees) do
      if not is_valid_login(login) then
        return false
      end
    end
  end
  if payload.post_create_blocked_by ~= nil then
    local post = payload.post_create_blocked_by
    if type(post) ~= "table"
      or not is_positive_integer(post.blocked_issue_number)
      or not is_bounded_marker_value(post.dedup_key, max_dedup_len) then
      return false
    end
  end
  local parent = normalize_parent_comment_target(payload.parent_comment_target)
  if parent == false then
    return false
  end
  return true
end

local function maybe_raise_post_create_blocked_by(payload, issue_number)
  local post = payload.post_create_blocked_by
  if post == nil then
    return
  end
  if not is_positive_integer(issue_number) then
    error("github-proxy: issue-create post_create_blocked_by missing created issue number")
  end
  raise("github_issue_blocked_by_request", {
    schema = "github-proxy.issue-blocked-by.v1",
    repo = payload.repo,
    blocked_issue_number = tonumber(post.blocked_issue_number),
    blocking_issue_number = tonumber(issue_number),
    dedup_key = tostring(post.dedup_key),
    source_ref = payload.source_ref,
  })
end

function M.write_issue_create_request(payload)
  if not M.validate_issue_create_payload(payload) then
    log.warn("github-proxy: issue-create request missing or invalid fields")
    return
  end

  local repo = payload.repo
  local mode = M.read_env("FKST_GITHUB_WRITE") == "1" and "real" or "dry-run"
  M.log_line("info", "github_issue_create", "OUTBOUND", {
    "mode=" .. mode,
    "repo=" .. tostring(repo),
    "dedup_key=" .. tostring(payload.dedup_key),
  })
  if mode ~= "real" then
    log.info("github-proxy dry-run: would create issue in " .. tostring(repo))
    return
  end

  local bot_login = M.assert_trusted_bot_configured()
  with_lock(M.issue_create_lock_key(payload.dedup_key), function()
    local parent = normalize_parent_comment_target(payload.parent_comment_target)
    if parent ~= nil then
      local parent_view = M.gh_exec(M.gh_issue_create_parent_view_cmd(parent), 30, "gh parent comment view")
      local parent_comments = M.parse_issue_comments(parent_view.stdout)
      local existing_created_issue = M.trusted_issue_created_number(parent_comments, payload.dedup_key, bot_login)
      if existing_created_issue ~= nil then
        log.info("github-proxy: skip-idempotent issue-create parent marker already present")
        maybe_raise_post_create_blocked_by(payload, existing_created_issue)
        return
      end
      if M.has_trusted_issue_create_intent_marker(parent_comments, payload.dedup_key, bot_login) then
        log.info("github-proxy: skip-idempotent issue-create parent marker already present")
        return
      end

      local intent_path = issue_create_intent_marker_body_file(payload.dedup_key)
      file.write(intent_path, M.issue_create_intent_marker(payload.dedup_key) .. "\n")
      M.gh_exec(M.gh_issue_create_parent_comment_cmd(parent, intent_path), 30, "gh parent issue-create intent comment")
      M.invalidate_entity_after_write(parent.repo, parent.kind, parent.number)
      local confirm = M.gh_exec(M.gh_issue_create_parent_view_cmd(parent), 30, "gh parent issue-create intent confirm")
      if not M.has_trusted_issue_create_intent_marker(M.parse_issue_comments(confirm.stdout), payload.dedup_key, bot_login) then
        error("github-proxy: issue-create intent marker not visible after write")
      end
    end

    -- once() is host-runtime scratch, not an external fact. Parent-backed
    -- requests use the parent intent marker as the cross-runtime dup gate. The
    -- no-parent fallback keeps issue body search as a bounded external backstop.
    local ran = once(M.issue_create_once_key(payload.dedup_key), function()
      if parent == nil then
        local search = M.gh_exec(M.gh_issue_create_search_cmd(repo, payload.dedup_key), 30, "gh issue list")
        if M.has_trusted_issue_create_marker(M.parse_issue_create_search(search.stdout), payload.dedup_key, bot_login) then
          log.info("github-proxy: skip-idempotent issue-create marker already present")
          return
        end
      end

      local body = tostring(payload.body) .. "\n\n" .. M.issue_create_marker(payload.dedup_key) .. "\n"
      body = M.with_github_debug_stamp(body, {
        emitter = "github-proxy.issue-create",
        target = "issue:" .. tostring(repo) .. "#new",
        dedup_key = payload.dedup_key,
      })
      local path = "/tmp/fkst-github-proxy-" .. issue_create_runtime_identity(payload.dedup_key) .. ".md"
      file.write(path, body)
      local created = M.gh_exec(M.gh_issue_create_cmd(repo, payload.title, path, payload.labels, payload.assignees), 30, "gh issue create")
      local issue_number = M.parse_created_issue_number(created.stdout)
      if issue_number ~= nil then
        M.invalidate_entity_after_write(repo, "issue", issue_number)
      end
      if parent ~= nil then
        local marker_path = issue_created_marker_body_file(payload.dedup_key)
        file.write(marker_path, M.issue_created_marker(payload.dedup_key, issue_number or "unknown") .. "\n")
        M.gh_exec(M.gh_issue_create_parent_comment_cmd(parent, marker_path), 30, "gh parent issue-created comment")
        M.invalidate_entity_after_write(parent.repo, parent.kind, parent.number)
      end
      maybe_raise_post_create_blocked_by(payload, issue_number)
    end)
    if not ran then
      log.info("github-proxy: skip-idempotent issue-create once marker already present")
    end
  end)
end
end

return S
