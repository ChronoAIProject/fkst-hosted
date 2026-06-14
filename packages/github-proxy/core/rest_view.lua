local S = {}

function S.install(M)
local function shell_single_quote(value)
  return "'" .. tostring(value):gsub("'", "'\\''") .. "'"
end

local function json_string(value)
  local text = tostring(value or "")
  text = text:gsub("\\", "\\\\")
  text = text:gsub('"', '\\"')
  text = text:gsub("\b", "\\b")
  text = text:gsub("\f", "\\f")
  text = text:gsub("\n", "\\n")
  text = text:gsub("\r", "\\r")
  text = text:gsub("\t", "\\t")
  text = text:gsub("[%z\1-\31]", function(char)
    return string.format("\\u%04X", string.byte(char))
  end)
  return '"' .. text .. '"'
end

local function json_value(value)
  if value == nil then
    return "null"
  end
  if type(value) == "boolean" then
    return value and "true" or "false"
  end
  if type(value) == "number" then
    return tostring(value)
  end
  return json_string(value)
end

local function rest_state(value)
  if value == nil then
    return nil
  end
  return tostring(value):upper()
end

local function rest_pr_state(pr)
  if type(pr) ~= "table" then
    return nil
  end
  local merged_at = pr.merged_at
  if pr.merged == true or (type(merged_at) == "string" and merged_at ~= "") then
    return "MERGED"
  end
  return rest_state(pr.state)
end

local function append_comments(target, value)
  if type(value) ~= "table" then
    return
  end
  if type(value.comments) == "table" then
    append_comments(target, value.comments)
    return
  end
  if value.id ~= nil or value.body ~= nil or value.user ~= nil or value.author ~= nil then
    table.insert(target, value)
    return
  end
  for _, item in ipairs(value) do
    append_comments(target, item)
  end
end

local function decode_json(stdout)
  local ok, decoded = pcall(json.decode, stdout)
  if ok and type(decoded) == "table" then
    return decoded
  end
  error("github-proxy: REST response is not valid JSON")
end

local function decode_entity_json(stdout)
  if stdout == nil or stdout == "" then
    error("github-proxy: REST entity response is empty")
  end
  return decode_json(stdout)
end

local function decode_comments_json(stdout)
  local source = stdout
  if source == nil or source == "" then
    source = "[]"
  end
  return decode_json(source)
end

local function labels_json(labels)
  local parts = {}
  for _, label in ipairs(labels or {}) do
    if type(label) == "table" then
      table.insert(parts, '{"name":' .. json_value(label.name) .. "}")
    elseif label ~= nil then
      table.insert(parts, '{"name":' .. json_value(label) .. "}")
    end
  end
  return "[" .. table.concat(parts, ",") .. "]"
end

local function assignees_json(assignees)
  local parts = {}
  for _, assignee in ipairs(assignees or {}) do
    if type(assignee) == "table" then
      table.insert(parts, '{"login":' .. json_value(assignee.login) .. "}")
    elseif assignee ~= nil then
      table.insert(parts, '{"login":' .. json_value(assignee) .. "}")
    end
  end
  return "[" .. table.concat(parts, ",") .. "]"
end

local function comments_json(comments)
  local parts = {}
  for _, comment in ipairs(comments or {}) do
    if type(comment) == "table" then
      local author_login = nil
      if type(comment.author) == "table" then
        author_login = comment.author.login
      elseif type(comment.user) == "table" then
        author_login = comment.user.login
      end
      local id = comment.databaseId or comment.database_id or comment.id
      table.insert(parts, '{"id":' .. json_value(id)
        .. ',"body":' .. json_value(comment.body)
        .. ',"author":{"login":' .. json_value(author_login) .. "}}")
    end
  end
  return "[" .. table.concat(parts, ",") .. "]"
end

local function object_field(name, value)
  if value == nil then
    return ""
  end
  return ',"' .. tostring(name) .. '":' .. json_value(value)
end

local function repo_name_with_owner(repo)
  if type(repo) ~= "table" then
    return nil
  end
  if repo.full_name ~= nil and tostring(repo.full_name) ~= "" then
    return tostring(repo.full_name)
  end
  if repo.nameWithOwner ~= nil and tostring(repo.nameWithOwner) ~= "" then
    return tostring(repo.nameWithOwner)
  end
  if type(repo.owner) == "table" and repo.owner.login ~= nil and repo.name ~= nil then
    return tostring(repo.owner.login) .. "/" .. tostring(repo.name)
  end
  return nil
end

local function repo_owner_login(repo)
  if type(repo) == "table" and type(repo.owner) == "table" and repo.owner.login ~= nil then
    return tostring(repo.owner.login)
  end
  local name_with_owner = repo_name_with_owner(repo)
  return name_with_owner and name_with_owner:match("^([^/]+)/") or nil
end

function M.gh_issue_rest_view_cmd(repo, issue_number)
  return "gh api " .. shell_single_quote("repos/" .. tostring(repo) .. "/issues/" .. tostring(issue_number))
end

function M.gh_pr_rest_view_cmd(repo, pr_number)
  return "gh api " .. shell_single_quote("repos/" .. tostring(repo) .. "/pulls/" .. tostring(pr_number))
end

function M.gh_issue_comments_api_cmd(repo, issue_number)
  return "gh api --paginate --slurp "
    .. shell_single_quote("repos/" .. tostring(repo) .. "/issues/" .. tostring(issue_number) .. "/comments?per_page=100")
end

function M.rest_comments_to_view_json(comments_stdout)
  local decoded = decode_comments_json(comments_stdout)
  local comments = {}
  append_comments(comments, decoded)
  return comments_json(comments)
end

function M.rest_issue_to_view_json(issue_stdout, comments_stdout)
  local issue = decode_entity_json(issue_stdout)
  local comment_source = comments_stdout
  if type(issue.comments) == "table" then
    comment_source = '{"comments":' .. comments_json(issue.comments) .. "}"
  end
  return '{"title":' .. json_value(issue.title)
    .. ',"body":' .. json_value(issue.body)
    .. ',"labels":' .. labels_json(issue.labels)
    .. ',"state":' .. json_value(rest_state(issue.state))
    .. ',"updatedAt":' .. json_value(issue.updated_at or issue.updatedAt)
    .. ',"assignees":' .. assignees_json(issue.assignees)
    .. ',"comments":' .. M.rest_comments_to_view_json(comment_source)
    .. "}"
end

function M.rest_pr_to_view_json(pr_stdout, comments_stdout)
  local pr = decode_entity_json(pr_stdout)
  local head = type(pr.head) == "table" and pr.head or {}
  local base = type(pr.base) == "table" and pr.base or {}
  local head_repo = type(head.repo) == "table" and head.repo or nil
  local base_repo = type(base.repo) == "table" and base.repo or nil
  local head_name_with_owner = repo_name_with_owner(head_repo)
  local base_name_with_owner = repo_name_with_owner(base_repo)
  if head_name_with_owner == nil or base_name_with_owner == nil then
    error("github-proxy: REST PR view missing repository facts")
  end
  local is_cross_repository = tostring(head_name_with_owner):lower() ~= tostring(base_name_with_owner):lower()
  local comment_source = comments_stdout
  if type(pr.comments) == "table" then
    comment_source = '{"comments":' .. comments_json(pr.comments) .. "}"
  end
  local head_repository = "{}"
  if head_name_with_owner ~= nil then
    head_repository = '{"nameWithOwner":' .. json_value(head_name_with_owner) .. "}"
  end
  local head_repository_owner = "{}"
  local owner_login = repo_owner_login(head_repo)
  if owner_login ~= nil then
    head_repository_owner = '{"login":' .. json_value(owner_login) .. "}"
  end
  return '{"headRefName":' .. json_value(head.ref)
    .. ',"headRefOid":' .. json_value(head.sha)
    .. ',"baseRefName":' .. json_value(base.ref)
    .. ',"state":' .. json_value(rest_pr_state(pr))
    .. ',"updatedAt":' .. json_value(pr.updated_at or pr.updatedAt)
    .. ',"headRepository":' .. head_repository
    .. ',"headRepositoryOwner":' .. head_repository_owner
    .. object_field("isCrossRepository", is_cross_repository)
    .. ',"labels":' .. labels_json(pr.labels)
    .. ',"comments":' .. M.rest_comments_to_view_json(comment_source)
    .. "}"
end

function M.fetch_rest_issue_view(repo, issue_number)
  local issue = M.gh_exec(M.gh_issue_rest_view_cmd(repo, issue_number), 30, "gh issue REST view")
  if issue.exit_code ~= 0 then
    return issue
  end
  local comments = M.gh_exec(M.gh_issue_comments_api_cmd(repo, issue_number), 30, "gh issue comments")
  if comments.exit_code ~= 0 then
    return comments
  end
  local ok, view_json = pcall(M.rest_issue_to_view_json, issue.stdout, comments.stdout)
  if not ok then
    return {
      stdout = "",
      stderr = tostring(view_json),
      exit_code = 1,
    }
  end
  return {
    stdout = view_json,
    stderr = "",
    exit_code = 0,
  }
end

function M.fetch_rest_pr_view(repo, pr_number)
  local pr = M.gh_exec(M.gh_pr_rest_view_cmd(repo, pr_number), 30, "gh PR REST view")
  if pr.exit_code ~= 0 then
    return pr
  end
  local comments = M.gh_exec(M.gh_issue_comments_api_cmd(repo, pr_number), 30, "gh PR comments")
  if comments.exit_code ~= 0 then
    return comments
  end
  local ok, view_json = pcall(M.rest_pr_to_view_json, pr.stdout, comments.stdout)
  if not ok then
    return {
      stdout = "",
      stderr = tostring(view_json),
      exit_code = 1,
    }
  end
  return {
    stdout = view_json,
    stderr = "",
    exit_code = 0,
  }
end

end

return S
