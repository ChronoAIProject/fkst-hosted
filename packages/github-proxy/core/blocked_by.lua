local S = {}

function S.install(M)
local max_dedup_len = 512
local max_repo_len = 200

local function shell_single_quote(value)
  return "'" .. tostring(value):gsub("'", "'\\''") .. "'"
end

local function is_bounded_string(value, limit)
  return type(value) == "string" and value ~= "" and #value <= limit
end

local function is_marker_value(value)
  return is_bounded_string(value, max_dedup_len)
    and tostring(value):find('[<>"\r\n]') == nil
end

local function optional_marker_value(value)
  return value == nil or is_marker_value(value)
end

local function is_positive_integer(value)
  local n = tonumber(value)
  return n ~= nil and n >= 1 and n % 1 == 0 and n <= 2147483647
end

local function split_repo(repo)
  local owner, name = tostring(repo or ""):match("^([^/]+)/([^/]+)$")
  if owner == nil or owner == "" or name == nil or name == "" then
    return nil, nil
  end
  return owner, name
end

local function runtime_segment(value)
  local safe = tostring(value or ""):gsub("[^%w._-]", "_")
  safe = safe:gsub("_+", "_"):gsub("^_+", ""):gsub("_+$", "")
  return safe == "" and "empty" or safe
end

local function issue_author_login(comment)
  if type(comment) ~= "table" then
    return nil
  end
  if comment.author_login ~= nil then
    return tostring(comment.author_login)
  end
  if type(comment.author) == "table" and comment.author.login ~= nil then
    return tostring(comment.author.login)
  end
  if type(comment.user) == "table" and comment.user.login ~= nil then
    return tostring(comment.user.login)
  end
  return nil
end

function M.validate_issue_blocked_by_payload(payload)
  if type(payload) ~= "table" then
    return false
  end
  if payload.schema ~= "github-proxy.issue-blocked-by.v1" then
    return false
  end
  if not is_bounded_string(payload.repo, max_repo_len) or split_repo(payload.repo) == nil then
    return false
  end
  if not is_positive_integer(payload.blocked_issue_number)
    or not is_positive_integer(payload.blocking_issue_number) then
    return false
  end
  if not is_marker_value(payload.dedup_key) then
    return false
  end
  if not optional_marker_value(payload.external_effect_saga)
    or not optional_marker_value(payload.external_effect_step) then
    return false
  end
  if type(payload.source_ref) ~= "table"
    or not is_bounded_string(payload.source_ref.kind, 80)
    or not is_bounded_string(payload.source_ref.ref, 200) then
    return false
  end
  return true
end

function M.issue_blocked_by_lock_key(repo, issue_number)
  local key = "github-proxy/blocked-by/" .. runtime_segment(repo) .. "/issue/" .. runtime_segment(issue_number)
  if #key > 200 then
    return key:sub(1, 200)
  end
  return key
end

function M.gh_issue_node_id_cmd(repo, issue_number)
  return M.gh_issue_rest_view_cmd(repo, issue_number)
end

function M.parse_issue_node_id(stdout)
  local decoded = json.decode(stdout or "{}")
  local id = decoded.node_id or decoded.id
  if type(id) ~= "string" or id == "" or id:find("%s") ~= nil then
    return nil
  end
  return id
end

function M.gh_issue_blocked_by_cmd(repo, issue_number)
  local owner, name = split_repo(repo)
  if owner == nil or not is_positive_integer(issue_number) then
    error("github-proxy: invalid blockedBy query target")
  end
  local query = '{repository(owner:"' .. owner .. '",name:"' .. name
    .. '"){issue(number:' .. tostring(math.floor(tonumber(issue_number)))
    .. '){blockedBy(first:50){totalCount pageInfo{hasNextPage} nodes{number repository{nameWithOwner}}}}}}'
  return "gh api graphql -f query=" .. shell_single_quote(query)
end

function M.gh_add_blocked_by_cmd(blocked_id, blocking_id)
  -- GitHub's AddBlockedByInput accepts `issueId` (the blocked issue) and
  -- `blockingIssueId` (the blocker); `blockedIssueId` is NOT a valid field and
  -- makes every addBlockedBy mutation fail (the fork-and-block "block" half then
  -- never lands, so a peer instance is never held at its dependency gate).
  local query = "mutation($b:ID!,$g:ID!){addBlockedBy(input:{issueId:$b,blockingIssueId:$g}){clientMutationId}}"
  return "gh api graphql -f query=" .. shell_single_quote(query)
    .. " -f b=" .. shell_single_quote(blocked_id)
    .. " -f g=" .. shell_single_quote(blocking_id)
end

local function parse_blocked_by(stdout)
  local decoded = json.decode(stdout or "{}")
  local issue = decoded.data
    and decoded.data.repository
    and decoded.data.repository.issue
  local blocked_by = issue and issue.blockedBy
  local nodes = blocked_by and blocked_by.nodes
  if type(nodes) ~= "table" then
    error("github-proxy: malformed blockedBy response")
  end
  if (type(blocked_by.totalCount) == "number" and blocked_by.totalCount > #nodes)
    or (type(blocked_by.pageInfo) == "table" and blocked_by.pageInfo.hasNextPage == true) then
    error("github-proxy: blockedBy response truncated")
  end
  local edges = {}
  for _, node in ipairs(nodes) do
    if type(node) ~= "table" or not is_positive_integer(node.number) then
      error("github-proxy: malformed blockedBy node")
    end
    local node_repo = node.repository and node.repository.nameWithOwner
    if type(node_repo) ~= "string" or node_repo == "" then
      error("github-proxy: malformed blockedBy node repository")
    end
    table.insert(edges, {
      repo = node_repo,
      number = tonumber(node.number),
    })
  end
  return edges
end

function M.issue_blocked_by_edge_exists(repo, blocked_issue_number, blocking_issue_number)
  local view = M.gh_exec(M.gh_issue_blocked_by_cmd(repo, blocked_issue_number), 30, "gh blockedBy view")
  for _, edge in ipairs(parse_blocked_by(view.stdout)) do
    if tostring(edge.repo) == tostring(repo) and tonumber(edge.number) == tonumber(blocking_issue_number) then
      return true
    end
  end
  return false
end

function M.blocked_by_marker(dedup_key, blocked_issue_number, blocking_issue_number)
  if not is_marker_value(dedup_key)
    or not is_positive_integer(blocked_issue_number)
    or not is_positive_integer(blocking_issue_number) then
    error("github-proxy: invalid blocked-by marker fields")
  end
  return '<!-- fkst:github-proxy:blocked-by:v1 dedup="' .. tostring(dedup_key)
    .. '" blocked="' .. tostring(math.floor(tonumber(blocked_issue_number)))
    .. '" blocking="' .. tostring(math.floor(tonumber(blocking_issue_number)))
    .. '" -->'
end

function M.has_trusted_blocked_by_marker(comments, dedup_key, bot_login)
  if type(comments) ~= "table" then
    return false
  end
  local marker_pattern = "<!%-%- fkst:github%-proxy:blocked%-by:v1.-%-%->"
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

local function marker_file(dedup_key)
  return "/tmp/fkst-github-proxy-blocked-by-" .. runtime_segment(dedup_key) .. ".md"
end

function M.write_issue_blocked_by_request(payload)
  if not M.validate_issue_blocked_by_payload(payload) then
    error("github-proxy: issue-blocked-by request missing or invalid fields")
  end
  local mode = M.read_env("FKST_GITHUB_WRITE") == "1" and "real" or "dry-run"
  M.log_line("info", "github_issue_blocked_by", "OUTBOUND", {
    "mode=" .. mode,
    "repo=" .. tostring(payload.repo),
    "blocked=" .. tostring(payload.blocked_issue_number),
    "blocking=" .. tostring(payload.blocking_issue_number),
    "dedup_key=" .. tostring(payload.dedup_key),
  })
  if mode ~= "real" then
    log.info("github-proxy dry-run: would add blockedBy edge")
    return
  end

  local bot_login = M.assert_trusted_bot_configured()
  with_lock(M.issue_blocked_by_lock_key(payload.repo, payload.blocked_issue_number), function()
    local comments_view = M.gh_exec(M.gh_issue_view_comments_cmd(payload.repo, payload.blocked_issue_number), 30, "gh issue comments")
    if M.has_trusted_blocked_by_marker(M.parse_issue_comments(comments_view.stdout), payload.dedup_key, bot_login) then
      log.info("github-proxy: skip-idempotent blocked-by marker already present")
      return
    end
    if not M.issue_blocked_by_edge_exists(payload.repo, payload.blocked_issue_number, payload.blocking_issue_number) then
      local blocked = M.gh_exec(M.gh_issue_node_id_cmd(payload.repo, payload.blocked_issue_number), 30, "gh blocked issue id")
      local blocking = M.gh_exec(M.gh_issue_node_id_cmd(payload.repo, payload.blocking_issue_number), 30, "gh blocking issue id")
      local blocked_id = M.parse_issue_node_id(blocked.stdout)
      local blocking_id = M.parse_issue_node_id(blocking.stdout)
      if blocked_id == nil or blocking_id == nil then
        error("github-proxy: issue node id missing")
      end
      M.gh_exec(M.gh_add_blocked_by_cmd(blocked_id, blocking_id), 30, "gh addBlockedBy")
    end

    local path = marker_file(payload.dedup_key)
    file.write(path, M.blocked_by_marker(payload.dedup_key, payload.blocked_issue_number, payload.blocking_issue_number) .. "\n")
    M.gh_exec(M.gh_issue_comment_cmd(payload.repo, payload.blocked_issue_number, path), 30, "gh blocked-by marker comment")
    M.invalidate_entity_after_write(payload.repo, "issue", payload.blocked_issue_number)
  end)
end

end

return S
