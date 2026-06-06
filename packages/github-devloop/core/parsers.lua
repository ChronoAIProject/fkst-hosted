local S = {}

function S.install(M)
function M.parse_issue_view_body(stdout)
  local decoded = json.decode(stdout or "{}")
  return M.bounded_body(decoded.body)
end

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
    labels = labels,
    comments = M.comments_from_json(decoded.comments),
    state = decoded.state,
  }
end

function M.comments_from_json(comments_json)
  local comments = {}
  for _, comment in ipairs(comments_json or {}) do
    if type(comment) == "table" and comment.body ~= nil then
      local author_login = nil
      if type(comment.author) == "table" and comment.author.login ~= nil then
        author_login = tostring(comment.author.login)
      elseif comment.author_login ~= nil then
        author_login = tostring(comment.author_login)
      end
      table.insert(comments, {
        body = tostring(comment.body),
        author_login = author_login,
        created_at = comment.createdAt or comment.created_at,
      })
    elseif type(comment) == "string" then
      table.insert(comments, {
        body = comment,
        author_login = M._test_bot_login,
      })
    end
  end
  return comments
end

function M.parse_issue_view_result(stdout)
  local decoded = json.decode(stdout or "{}")
  local state = M.issue_state_from_json(decoded)

  return {
    labels = state.labels,
    comments = state.comments,
  }
end

function M.parse_issue_view_loop(stdout)
  local decoded = json.decode(stdout or "{}")
  local result = M.parse_issue_view_result(stdout)
  return {
    title = tostring(decoded.title or ""),
    body = M.bounded_body(decoded.body),
    updated_at = decoded.updatedAt or decoded.updated_at,
    state = decoded.state,
    labels = result.labels,
    comments = result.comments,
  }
end

function M.parse_issue_view_meta(stdout)
  local decoded = json.decode(stdout or "{}")
  local result = M.parse_issue_view_result(stdout)
  return {
    title = tostring(decoded.title or ""),
    body = M.bounded_body(decoded.body),
    labels = result.labels,
    comments = result.comments,
  }
end

function M.parse_issue_view_implement(stdout)
  return M.parse_issue_view_meta(stdout)
end

function M.parse_issue_view_open_pr(stdout)
  local decoded = json.decode(stdout or "{}")
  local result = M.parse_issue_view_result(stdout)
  return {
    title = tostring(decoded.title or ""),
    labels = result.labels,
    comments = result.comments,
  }
end

function M.parse_issue_view_reviewing(stdout)
  return M.parse_issue_view_result(stdout)
end

function M.parse_issue_view_review(stdout)
  return M.parse_issue_view_meta(stdout)
end

function M.parse_issue_view_fix(stdout)
  return M.parse_issue_view_meta(stdout)
end

function M.parse_issue_view_review_loop(stdout)
  return M.parse_issue_view_meta(stdout)
end

function M.parse_issue_view_review_meta(stdout)
  return M.parse_issue_view_meta(stdout)
end

function M.parse_issue_view_merge(stdout)
  local decoded = json.decode(stdout or "{}")
  local result = M.parse_issue_view_meta(stdout)
  result.state = decoded.state
  return result
end

local function repository_name_with_owner(head_repository, head_repository_owner)
  if type(head_repository) == "string" then
    return head_repository
  end
  if type(head_repository) ~= "table" then
    return nil
  end
  if head_repository.nameWithOwner ~= nil then
    return tostring(head_repository.nameWithOwner)
  end
  if head_repository.name_with_owner ~= nil then
    return tostring(head_repository.name_with_owner)
  end
  local name = head_repository.name
  local owner = nil
  if type(head_repository.owner) == "table" and head_repository.owner.login ~= nil then
    owner = head_repository.owner.login
  elseif type(head_repository_owner) == "table" and head_repository_owner.login ~= nil then
    owner = head_repository_owner.login
  elseif type(head_repository_owner) == "string" then
    owner = head_repository_owner
  end
  if owner ~= nil and name ~= nil then
    return tostring(owner) .. "/" .. tostring(name)
  end
  return nil
end

function M.parse_pr_view_origin(stdout)
  local decoded = json.decode(stdout or "{}")
  local head_repo = repository_name_with_owner(
    decoded.headRepository or decoded.head_repository,
    decoded.headRepositoryOwner or decoded.head_repository_owner
  )
  local is_cross_repository = decoded.isCrossRepository
  if is_cross_repository == nil then
    is_cross_repository = decoded.is_cross_repository
  end
  return {
    head_ref_name = decoded.headRefName or decoded.head_ref_name,
    head_sha = decoded.headRefOid or decoded.head_ref_oid,
    state = decoded.state,
    comments = M.comments_from_json(decoded.comments),
    head_repository = head_repo,
    is_cross_repository = is_cross_repository,
  }
end

function M.parse_pr_view_fix(stdout)
  return M.parse_pr_view_origin(stdout)
end

local function status_rollup_entries(value)
  if type(value) ~= "table" then
    return {}
  end
  if type(value.nodes) == "table" then
    return value.nodes
  end
  return value
end

local function review_entries(value)
  if type(value) ~= "table" then
    return {}
  end
  if type(value.nodes) == "table" then
    return value.nodes
  end
  return value
end

local function review_commit_id(review)
  if type(review) ~= "table" then
    return nil
  end
  local commit = review.commit_id or review.commitId or review.commitOID or review.commitOid or review.commit
  if type(commit) == "table" then
    commit = commit.oid or commit.id
  end
  if M._is_git_sha(commit) then
    return tostring(commit)
  end
  return nil
end

function M.parse_pr_view_merge(stdout)
  local decoded = json.decode(stdout or "{}")
  local result = M.parse_pr_view_origin(stdout)
  result.mergeable = decoded.mergeable
  result.merge_state_status = decoded.mergeStateStatus or decoded.merge_state_status
  result.status_check_rollup = status_rollup_entries(decoded.statusCheckRollup or decoded.status_check_rollup)
  result.merged_at = decoded.mergedAt or decoded.merged_at
  result.latest_reviews = review_entries(decoded.latestReviews or decoded.latest_reviews)
  return result
end

function M.parse_pr_view_head_state(stdout)
  local decoded = json.decode(stdout or "{}")
  return {
    head_ref_name = decoded.headRefName or decoded.head_ref_name,
    state = decoded.state,
  }
end

local function comment_body(comment)
  if type(comment) == "table" then
    return tostring(comment.body or "")
  end
  return tostring(comment or "")
end

local function comment_author_login(comment)
  if type(comment) == "table" then
    return comment.author_login
  end
  return M._test_bot_login
end

local function comment_created_at(comment)
  if type(comment) == "table" then
    return comment.created_at
  end
  return nil
end

local function is_trusted_comment(comment)
  return comment_author_login(comment) == M.trusted_bot_login()
end

local function trusted_marker_comments(comments)
  local filtered = {}
  if type(comments) ~= "table" then
    return filtered
  end
  for _, comment in ipairs(comments) do
    if is_trusted_comment(comment) then
      table.insert(filtered, comment)
    end
  end
  return filtered
end

function M.comment_body(comment)
  return comment_body(comment)
end

function M.comment_author_login(comment)
  return comment_author_login(comment)
end

function M.comment_created_at(comment)
  return comment_created_at(comment)
end


M._comment_body = comment_body
M._comment_author_login = comment_author_login
M._comment_created_at = comment_created_at
M._is_trusted_comment = is_trusted_comment
M._trusted_marker_comments = trusted_marker_comments

local function upper_text(value)
  return tostring(value or ""):upper()
end

local function check_entry_state(entry)
  if type(entry) ~= "table" then
    return nil, nil
  end
  return upper_text(entry.state or entry.status), upper_text(entry.conclusion)
end

local green_check_conclusions = {
  SUCCESS = true,
  NEUTRAL = true,
  SKIPPED = true,
}

local green_status_states = {
  SUCCESS = true,
}

local red_status_states = {
  ERROR = true,
  FAILURE = true,
}

function M.pr_rollup_green(pr)
  local entries = type(pr) == "table" and pr.status_check_rollup or nil
  if type(entries) ~= "table" or #entries == 0 then
    return false, "missing-status-rollup"
  end
  for _, entry in ipairs(entries) do
    local state, conclusion = check_entry_state(entry)
    if state == "COMPLETED" then
      if not green_check_conclusions[conclusion] then
        return false, "rollup-red"
      end
    elseif conclusion == "" and green_status_states[state] then
      -- Legacy StatusContext entries report state=SUCCESS without a conclusion.
    elseif conclusion == "" and red_status_states[state] then
      return false, "rollup-red"
    else
      return false, "rollup-pending"
    end
  end
  return true, "rollup-green"
end

function M.pr_mergeable(pr)
  if type(pr) ~= "table" then
    return false, "missing-pr"
  end
  local mergeable = upper_text(pr.mergeable)
  local merge_state = upper_text(pr.merge_state_status)
  if mergeable == "UNKNOWN" then
    return false, "mergeable-unknown"
  end
  if mergeable ~= "MERGEABLE" then
    if mergeable == "" then
      return false, "missing-mergeability"
    end
    return false, "mergeable-" .. mergeable:lower()
  end
  if merge_state ~= "CLEAN" then
    if merge_state == "" then
      return false, "missing-mergeability"
    end
    return false, "merge-state-" .. merge_state:lower()
  end
  return true, "mergeable"
end

function M.is_ci_red_reason(reason)
  return tostring(reason or "") == "rollup-red"
end

function M.is_not_mergeable_reason(reason)
  local text = tostring(reason or "")
  return text == "mergeable-conflicting"
    or text == "mergeable-false"
    or text == "merge-state-dirty"
    or text == "merge-state-conflicting"
end

M._upper_text = upper_text
M._review_commit_id = review_commit_id
end

return S
