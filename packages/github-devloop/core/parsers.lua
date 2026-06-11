local S = {}

function S.install(M)
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
        id = comment.id,
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

local function label_names(labels_json)
  local labels = {}
  for _, label in ipairs(labels_json or {}) do
    if type(label) == "table" and label.name ~= nil then
      table.insert(labels, tostring(label.name))
    elseif type(label) == "string" then
      table.insert(labels, label)
    end
  end
  return labels
end

function M.parse_issue_list_intake(stdout)
  local decoded = json.decode(stdout or "[]")
  local issues = {}
  if type(decoded) ~= "table" then
    return issues
  end
  for _, issue in ipairs(decoded) do
    if type(issue) == "table" then
      table.insert(issues, {
        number = issue.number,
        title = tostring(issue.title or ""),
        updated_at = issue.updatedAt or issue.updated_at,
        labels = label_names(issue.labels),
      })
    end
  end
  return issues
end

local function each_paginated_item(decoded, callback)
  if type(decoded) ~= "table" then
    return
  end
  for _, value in ipairs(decoded) do
    if type(value) == "table" then
      if value[1] ~= nil then
        for _, item in ipairs(value) do
          callback(item)
        end
      else
        callback(value)
      end
    end
  end
end

local function parse_numbered_list(stdout)
  local decoded = json.decode(stdout or "[]")
  local items = {}
  each_paginated_item(decoded, function(item)
    if type(item) == "table" and tonumber(item.number) ~= nil then
      table.insert(items, {
        number = tonumber(item.number),
        state = item.state,
        updated_at = item.updated_at or item.updatedAt,
      })
    end
  end)
  return items
end

function M.parse_issue_list_observe(stdout)
  return parse_numbered_list(stdout)
end

function M.parse_dashboard_issue_list(stdout)
  local decoded = json.decode(stdout or "[]")
  local items = {}
  if type(decoded) ~= "table" then
    return items
  end
  each_paginated_item(decoded, function(issue)
    if type(issue) == "table" and tonumber(issue.number) ~= nil then
      local author_login = nil
      if type(issue.author) == "table" and issue.author.login ~= nil then
        author_login = tostring(issue.author.login)
      elseif issue.author_login ~= nil then
        author_login = tostring(issue.author_login)
      elseif type(issue.user) == "table" and issue.user.login ~= nil then
        author_login = tostring(issue.user.login)
      end
      table.insert(items, {
        number = tonumber(issue.number),
        title = tostring(issue.title or ""),
        author_login = author_login,
        body = tostring(issue.body or ""),
        labels = issue.labels,
        updated_at = issue.updated_at or issue.updatedAt,
      })
    end
  end)
  return items
end

function M.parse_repo_labels(stdout)
  local decoded = json.decode(stdout or "[]")
  local items = {}
  each_paginated_item(decoded, function(label)
    if type(label) == "table" and label.name ~= nil then
      table.insert(items, {
        name = tostring(label.name),
        color = label.color and tostring(label.color) or nil,
        description = label.description and tostring(label.description) or nil,
      })
    end
  end)
  return items
end

function M.parse_pr_list_observe(stdout)
  return parse_numbered_list(stdout)
end

function M.parse_pr_list_freshness(stdout)
  local decoded = json.decode(stdout or "[]")
  local prs = {}
  each_paginated_item(decoded, function(pr)
    if type(pr) == "table" and tonumber(pr.number) ~= nil then
      table.insert(prs, {
        number = tonumber(pr.number),
        state = pr.state,
        updated_at = pr.updated_at or pr.updatedAt,
        head_sha = pr.headRefOid or pr.head_ref_oid,
        head_ref_name = pr.headRefName or pr.head_ref_name,
        base_ref_name = pr.baseRefName or pr.base_ref_name,
        is_draft = pr.isDraft or pr.is_draft,
      })
    end
  end)
  return prs
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
    updated_at = decoded.updatedAt or decoded.updated_at,
    state = decoded.state,
    labels = result.labels,
    comments = result.comments,
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

function M.parse_issue_view_decompose(stdout)
  local decoded = json.decode(stdout or "{}")
  local result = M.parse_issue_view_result(stdout)
  return {
    title = tostring(decoded.title or ""),
    body = tostring(decoded.body or ""),
    labels = result.labels,
    comments = result.comments,
  }
end

function M.parse_issue_view_fix(stdout)
  return M.parse_issue_view_meta(stdout)
end

function M.parse_issue_view_review_loop(stdout)
  return M.parse_issue_view_meta(stdout)
end

function M.parse_issue_view_merge(stdout)
  local decoded = json.decode(stdout or "{}")
  local result = M.parse_issue_view_meta(stdout)
  result.state = decoded.state
  return result
end

function M.parse_issue_view_observe(stdout)
  local decoded = json.decode(stdout or "{}")
  return {
    title = tostring(decoded.title or ""),
    state = decoded.state,
    comments = M.comments_from_json(decoded.comments),
  }
end

local function repository_name_with_owner(head_repository, head_repository_owner)
  if type(head_repository) == "string" then
    return head_repository
  end
  if type(head_repository) ~= "table" then
    return nil
  end
  if head_repository.nameWithOwner ~= nil and head_repository.nameWithOwner ~= "" then
    return tostring(head_repository.nameWithOwner)
  end
  if head_repository.name_with_owner ~= nil and head_repository.name_with_owner ~= "" then
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
    base_ref_name = decoded.baseRefName or decoded.base_ref_name,
    state = decoded.state,
    updated_at = decoded.updatedAt or decoded.updated_at,
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

function M.parse_pr_view_merge(stdout)
  local decoded = json.decode(stdout or "{}")
  local result = M.parse_pr_view_origin(stdout)
  result.is_draft = decoded.isDraft
  if result.is_draft == nil then
    result.is_draft = decoded.is_draft
  end
  result.mergeable = decoded.mergeable
  result.merge_state_status = decoded.mergeStateStatus or decoded.merge_state_status
  result.status_check_rollup = status_rollup_entries(decoded.statusCheckRollup or decoded.status_check_rollup)
  result.merged_at = decoded.mergedAt or decoded.merged_at
  result.labels = label_names(decoded.labels)
  return result
end

function M.parse_pr_list_head_base(stdout)
  local decoded = json.decode(stdout or "[]")
  local prs = {}
  if type(decoded) ~= "table" then
    return prs
  end
  for _, pr in ipairs(decoded) do
    if type(pr) == "table" then
      table.insert(prs, {
        number = pr.number,
        head_sha = pr.headRefOid or pr.head_ref_oid,
        head_ref_name = pr.headRefName or pr.head_ref_name,
        base_ref_name = pr.baseRefName or pr.base_ref_name,
        state = pr.state,
      })
    end
  end
  return prs
end

local function check_run_entries(value)
  if type(value) ~= "table" then
    return {}
  end
  if type(value.check_runs) == "table" then
    return value.check_runs
  end
  return value
end

function M.parse_commit_check_runs(stdout)
  local decoded = json.decode(stdout or "{}")
  local runs = {}
  for _, run in ipairs(check_run_entries(decoded)) do
    if type(run) == "table" then
      table.insert(runs, {
        name = run.name,
        status = run.status,
        conclusion = run.conclusion,
      })
    end
  end
  return runs
end

function M.parse_pr_view_head_state(stdout)
  local decoded = json.decode(stdout or "{}")
  return {
    head_ref_name = decoded.headRefName or decoded.head_ref_name,
    base_ref_name = decoded.baseRefName or decoded.base_ref_name,
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
  -- NEUTRAL is excluded for irreversible-merge safety.
  SKIPPED = true,
}

local green_status_states = {
  SUCCESS = true,
}

local green_check_run_conclusions = {
  SUCCESS = true,
  NEUTRAL = true,
  SKIPPED = true,
}

local red_status_states = {
  ERROR = true,
  FAILURE = true,
}

local required_check_run_names = {
  "test",
}

local required_check_run_name_set = {}
for _, name in ipairs(required_check_run_names) do
  required_check_run_name_set[name] = true
end

local max_rollup_check_name_len = 80
local max_rollup_failure_summary_len = 200
local max_rollup_failure_checks = 3

local function safe_rollup_check_name(M, entry)
  local name = "unknown"
  if type(entry) == "table" then
    name = tostring(entry.name or entry.context or entry.workflowName or entry.workflow_name or "")
    if name == "" then
      name = "unknown"
    end
  end
  name = M.neutralize_untrusted_comment_text(M._neutralize_fkst_markers(name))
  name = M._one_line(name):gsub("[%c]", " "):gsub("^%s+", ""):gsub("%s+$", "")
  if name == "" then
    name = "unknown"
  end
  if #name > max_rollup_check_name_len then
    name = name:sub(1, max_rollup_check_name_len)
  end
  return name
end

local function check_name(entry)
  if type(entry) ~= "table" then
    return ""
  end
  return tostring(entry.name or entry.context or entry.workflowName or entry.workflow_name or "")
end

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

function M.commit_check_runs_green(runs)
  if type(runs) ~= "table" or #runs == 0 then
    return false, "missing-status-rollup"
  end
  local seen_required = {}
  for _, run in ipairs(runs) do
    local name = check_name(run)
    if required_check_run_name_set[name] then
      seen_required[name] = true
      local state, conclusion = check_entry_state(run)
      if state == "COMPLETED" then
        if not green_check_run_conclusions[conclusion] then
          return false, "rollup-red"
        end
      else
        return false, "rollup-pending"
      end
    end
  end
  for _, name in ipairs(required_check_run_names) do
    if not seen_required[name] then
      return false, "missing-status-rollup"
    end
  end
  return true, "rollup-green"
end

function M.pr_rollup_failure_summary(pr)
  local entries = type(pr) == "table" and pr.status_check_rollup or nil
  if type(entries) ~= "table" or #entries == 0 then
    return ""
  end
  local failed = {}
  local failed_total = 0
  for _, entry in ipairs(entries) do
    local state, conclusion = check_entry_state(entry)
    local is_failed = false
    if state == "COMPLETED" then
      is_failed = not green_check_conclusions[conclusion]
    elseif conclusion == "" and red_status_states[state] then
      is_failed = true
    end
    if is_failed then
      local status = state
      if conclusion ~= "" then
        status = status .. "/" .. conclusion
      end
      failed_total = failed_total + 1
      if #failed < max_rollup_failure_checks then
        table.insert(failed, safe_rollup_check_name(M, entry) .. ": " .. status)
      end
    end
  end
  if #failed == 0 then
    return ""
  end
  local summary = table.concat(failed, "; ")
  if failed_total > #failed then
    local suffix = "; (+" .. tostring(failed_total - #failed) .. " more)"
    local head_limit = max_rollup_failure_summary_len - #suffix
    if head_limit < 1 then
      head_limit = 1
    end
    if #summary > head_limit then
      summary = summary:sub(1, head_limit):gsub(";?%s*$", "")
    end
    summary = summary .. suffix
  end
  if #summary > max_rollup_failure_summary_len then
    summary = summary:sub(1, max_rollup_failure_summary_len)
  end
  summary = summary:gsub("[%c]", " "):gsub("^%s+", ""):gsub("%s+$", "")
  return summary
end

function M.rollup_red_fix_reason(pr, reason)
  local base_reason = tostring(reason or "rollup-red")
  local failure_summary = M.pr_rollup_failure_summary(pr)
  if failure_summary == "" then
    return base_reason
  end
  return base_reason .. ": " .. failure_summary
end

M._max_rollup_check_name_len = max_rollup_check_name_len
M._max_rollup_failure_summary_len = max_rollup_failure_summary_len
M._required_check_run_names = required_check_run_names

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
end

return S
