local S = {}

function S.install(M, shared)
local strings = require("forge.strings")
local each_paginated_item = shared.each_paginated_item

function M.comments_from_json(comments_json)
  local comments = {}
  for _, comment in ipairs(comments_json or {}) do
    if type(comment) == "table" and comment.body ~= nil then
      local author_login = nil
      if type(comment.author) == "table" and comment.author.login ~= nil then
        author_login = tostring(comment.author.login)
      elseif type(comment.user) == "table" and comment.user.login ~= nil then
        author_login = tostring(comment.user.login)
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
        id = run.id,
        databaseId = run.databaseId,
        database_id = run.database_id,
        name = run.name,
        status = run.status,
        conclusion = run.conclusion,
        head_sha = run.head_sha,
        headSha = run.headSha,
        check_suite = run.check_suite,
        checkSuite = run.checkSuite,
      })
    end
  end
  return runs
end

local function comment_author_login(comment)
  -- Normalize the comment author login so an author read as "<slug>[bot]" (REST)
  -- matches a bare-"<slug>" configured bot login (GraphQL). No-op for ordinary logins.
  if type(comment) == "table" then
    if comment.author_login ~= nil then
      return M.strip_bot_login_suffix(comment.author_login)
    end
    if type(comment.author) == "table" and comment.author.login ~= nil then
      return M.strip_bot_login_suffix(comment.author.login)
    end
    if type(comment.user) == "table" and comment.user.login ~= nil then
      return M.strip_bot_login_suffix(comment.user.login)
    end
    return nil
  end
  return M._test_bot_login
end

local function comment_created_at(comment)
  if type(comment) == "table" then
    return comment.created_at
  end
  return nil
end

local function is_trusted_comment(comment, trust_set)
  -- Parser-only trust filtering keeps the test default; pre-assert ownership gates use claim_owner.
  local author = comment_author_login(comment)
  if type(trust_set) == "table" then
    return trust_set[author] == true
  end
  return author == M.trusted_bot_login()
end

local function trusted_marker_comments(comments, trust_set)
  local filtered = {}
  if type(comments) ~= "table" then
    return filtered
  end
  for _, comment in ipairs(comments) do
    if is_trusted_comment(comment, trust_set) then
      table.insert(filtered, comment)
    end
  end
  return filtered
end

function M.comment_body(comment)
  return strings.comment_body(comment)
end

function M.comment_author_login(comment)
  return comment_author_login(comment)
end

function M.comment_created_at(comment)
  return comment_created_at(comment)
end


M._comment_body = strings.comment_body
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

local function entry_commit_sha(entry)
  if type(entry) ~= "table" then
    return nil
  end
  local candidates = {
    entry.headSha,
    entry.head_sha,
    entry.sha,
    entry.oid,
  }
  if type(entry.commit) == "table" then
    table.insert(candidates, entry.commit.oid)
    table.insert(candidates, entry.commit.sha)
  end
  if type(entry.checkSuite) == "table" then
    table.insert(candidates, entry.checkSuite.headSha)
    table.insert(candidates, entry.checkSuite.head_sha)
    if type(entry.checkSuite.commit) == "table" then
      table.insert(candidates, entry.checkSuite.commit.oid)
      table.insert(candidates, entry.checkSuite.commit.sha)
    end
  end
  if type(entry.check_suite) == "table" then
    table.insert(candidates, entry.check_suite.headSha)
    table.insert(candidates, entry.check_suite.head_sha)
    if type(entry.check_suite.commit) == "table" then
      table.insert(candidates, entry.check_suite.commit.oid)
      table.insert(candidates, entry.check_suite.commit.sha)
    end
  end
  for _, candidate in ipairs(candidates) do
    if M._is_git_sha(candidate) then
      return tostring(candidate)
    end
  end
  return nil
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

function M.rollup_failure_gate_sha(pr)
  local entries = type(pr) == "table" and pr.status_check_rollup or nil
  if type(entries) ~= "table" or #entries == 0 then
    return nil
  end
  local gate_sha = nil
  for _, entry in ipairs(entries) do
    local state, conclusion = check_entry_state(entry)
    local is_failed = false
    if state == "COMPLETED" then
      is_failed = not green_check_conclusions[conclusion]
    elseif conclusion == "" and red_status_states[state] then
      is_failed = true
    end
    if is_failed then
      local sha = entry_commit_sha(entry)
      if sha == nil then
        return nil
      end
      if gate_sha == nil then
        gate_sha = sha
      elseif gate_sha ~= sha then
        return nil
      end
    end
  end
  return gate_sha
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
    if merge_state == "UNSTABLE" then
      local rollup_green, rollup_reason = M.pr_rollup_green(pr)
      if not rollup_green and (rollup_reason == "rollup-red" or rollup_reason == "rollup-pending") then
        return true, "mergeable"
      end
    end
    return false, "merge-state-" .. merge_state:lower()
  end
  return true, "mergeable"
end

function M.is_ci_red_reason(reason)
  return tostring(reason or "") == "own-ci-red"
end

function M.is_ci_wait_reason(reason)
  local text = tostring(reason or "")
  return text == "external-ci-red"
    or text == "integration-ci-red"
    or text == "ci-unknown"
    or text == "checks-pending"
    or text == "rollup-pending"
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
