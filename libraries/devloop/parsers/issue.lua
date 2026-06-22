local S = {}

function S.install(M, shared)
local label_names = shared.label_names
local each_paginated_item = shared.each_paginated_item
local parse_numbered_list = shared.parse_numbered_list

function M.parse_issue_view_state(stdout)
  local decoded = json.decode(stdout or "{}")
  return M.issue_state_from_json(decoded)
end

function M.issue_state_from_json(decoded)
  local labels = {}
  for _, label in ipairs(decoded.labels or {}) do
    if type(label) == "table" and label.name ~= nil then
      table.insert(labels, tostring(label.name))
    elseif type(label) == "string" then
      table.insert(labels, label)
    end
  end

  return {
    title = decoded.title ~= nil and tostring(decoded.title) or nil,
    updated_at = decoded.updatedAt or decoded.updated_at,
    labels = labels,
    comments = M.comments_from_json(decoded.comments),
    state = decoded.state,
    assignees = M.assignee_logins(decoded.assignees),
    author_login = M.issue_author_login(decoded),
  }
end

function M.parse_issue_list_intake(stdout, limit)
  local decoded = json.decode(stdout or "[]")
  local issues = {}
  if type(decoded) ~= "table" then
    return issues
  end
  local max_items = math.floor(tonumber(limit or 2147483647) or 2147483647)
  if max_items < 1 then
    return issues
  end
  each_paginated_item(decoded, function(issue)
    local number = type(issue) == "table" and tonumber(issue.number) or nil
    if number ~= nil and issue.pull_request == nil and #issues < max_items then
      table.insert(issues, {
        number = number,
        title = tostring(issue.title or ""),
        body = tostring(issue.body or ""),
        created_at = issue.createdAt or issue.created_at,
        updated_at = issue.updatedAt or issue.updated_at,
        labels = label_names(issue.labels),
        assignees = M.assignee_logins(issue.assignees),
        author_login = M.issue_author_login(issue),
      })
    end
  end)
  return issues
end

function M.parse_issue_list_recent_closed(stdout)
  local decoded = json.decode(stdout or "[]")
  local issues = {}
  if type(decoded) ~= "table" then
    error("github-devloop: recent closed issue list decode failed")
  end
  each_paginated_item(decoded, function(issue)
    local number = type(issue) == "table" and tonumber(issue.number) or nil
    local title = type(issue) == "table" and issue.title or nil
    local closed_at = type(issue) == "table" and (issue.closedAt or issue.closed_at) or nil
    if number == nil or title == nil or closed_at == nil or type(issue.labels) ~= "table" then
      error("github-devloop: recent closed issue list item missing required fields")
    end
    table.insert(issues, {
      number = number,
      title = tostring(title),
      closed_at = tostring(closed_at),
      closedAt = tostring(closed_at),
      labels = label_names(issue.labels),
    })
  end)
  return issues
end

function M.parse_issue_number_list(stdout)
  local decoded = json.decode(stdout or "[]")
  local issues = {}
  if type(decoded) ~= "table" then
    return issues
  end
  each_paginated_item(decoded, function(issue)
    local number = type(issue) == "table" and tonumber(issue.number) or nil
    if number ~= nil then
      table.insert(issues, {
        number = number,
      })
    end
  end)
  return issues
end

function M.parse_issue_list_observe(stdout)
  return parse_numbered_list(stdout)
end

function M.parse_issue_view_result(stdout)
  local decoded = json.decode(stdout or "{}")
  local state = M.issue_state_from_json(decoded)

  return {
    labels = state.labels,
    comments = state.comments,
    assignees = M.assignee_logins(decoded.assignees),
    author_login = state.author_login,
  }
end

function M.parse_issue_view_loop(stdout)
  local decoded = json.decode(stdout or "{}")
  local result = M.parse_issue_view_result(stdout)
  return {
    title = tostring(decoded.title or ""),
    updated_at = decoded.updatedAt or decoded.updated_at,
    state = decoded.state,
    labels = result.labels,
    comments = result.comments,
    assignees = result.assignees,
    author_login = result.author_login,
  }
end

function M.parse_issue_view_intake_scan(stdout)
  return M.parse_issue_view_state(stdout)
end

function M.parse_issue_view_intake_judge(stdout)
  local decoded = json.decode(stdout or "{}")
  local result = M.parse_issue_view_result(stdout)
  return {
    title = tostring(decoded.title or ""),
    body = tostring(decoded.body or ""),
    updated_at = decoded.updatedAt or decoded.updated_at,
    state = decoded.state,
    labels = result.labels,
    comments = result.comments,
    assignees = result.assignees,
    author_login = result.author_login,
  }
end

function M.parse_issue_view_meta(stdout)
  local decoded = json.decode(stdout or "{}")
  local result = M.parse_issue_view_result(stdout)
  return {
    title = tostring(decoded.title or ""),
    labels = result.labels,
    comments = result.comments,
  }
end

function M.parse_issue_view_implement(stdout)
  local decoded = json.decode(stdout or "{}")
  local result = M.parse_issue_view_meta(stdout)
  result.body = tostring(decoded.body or "")
  result.state = decoded.state
  result.author_login = M.issue_author_login(decoded)
  return result
end

function M.parse_issue_view_open_pr(stdout)
  local decoded = json.decode(stdout or "{}")
  local result = M.parse_issue_view_result(stdout)
  return {
    title = tostring(decoded.title or ""),
    labels = result.labels,
    comments = result.comments,
    assignees = result.assignees,
    author_login = result.author_login,
  }
end

function M.parse_issue_view_reviewing(stdout)
  return M.parse_issue_view_result(stdout)
end

function M.parse_issue_view_review(stdout)
  local decoded = json.decode(stdout or "{}")
  local result = M.parse_issue_view_meta(stdout)
  result.assignees = M.assignee_logins(decoded.assignees)
  result.author_login = M.issue_author_login(decoded)
  return result
end

function M.parse_issue_view_decompose(stdout)
  local decoded = json.decode(stdout or "{}")
  local result = M.parse_issue_view_result(stdout)
  return {
    title = tostring(decoded.title or ""),
    body = tostring(decoded.body or ""),
    labels = result.labels,
    comments = result.comments,
    assignees = result.assignees,
    author_login = result.author_login,
  }
end

function M.parse_issue_view_fix(stdout)
  return M.parse_issue_view_meta(stdout)
end

function M.parse_issue_view_review_loop(stdout)
  local decoded = json.decode(stdout or "{}")
  local result = M.parse_issue_view_meta(stdout)
  result.assignees = M.assignee_logins(decoded.assignees)
  result.author_login = M.issue_author_login(decoded)
  return result
end

function M.parse_issue_view_merge(stdout)
  local decoded = json.decode(stdout or "{}")
  local result = M.parse_issue_view_result(stdout)
  result.title = tostring(decoded.title or "")
  result.state = decoded.state
  return result
end

function M.parse_issue_view_observe(stdout)
  local decoded = json.decode(stdout or "{}")
  return {
    title = tostring(decoded.title or ""),
    state = decoded.state,
    state_reason = decoded.stateReason or decoded.state_reason,
    comments = M.comments_from_json(decoded.comments),
    assignees = M.assignee_logins(decoded.assignees),
    author_login = M.issue_author_login(decoded),
  }
end
end

return S
