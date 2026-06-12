local t = fkst.test
local core = require("core")

local function nonce()
  return tostring({}):gsub("[^%w._-]", "_")
end

local function issue_list_json(updated_at, state)
  return string.format(
    '[[{"number":42,"title":"Bridge issue","html_url":"https://github.example/owner/x/issues/42","updated_at":"%s","state":"%s","labels":[{"name":"fkst-dev:enabled"},{"name":"bug"}]}]]\n',
    updated_at or "2026-06-03T01:02:03Z",
    state or "open"
  )
end

local function pr_list_json(updated_at, state)
  return string.format(
    '[[{"number":7,"title":"Bridge PR","html_url":"https://github.example/owner/x/pull/7","updated_at":"%s","state":"%s","labels":[{"name":"review"}]}]]\n',
    updated_at or "2026-06-03T02:03:04Z",
    state or "open"
  )
end

local function runtime_root(name)
  return "/tmp/fkst-packages-test/github-proxy/" .. tostring(now()) .. "/" .. nonce() .. "/" .. name
end

local function base_env(name, extra)
  local env = {
    FKST_GITHUB_REPO = "owner/x",
    FKST_RUNTIME_ROOT = runtime_root(name),
  }
  for key, value in pairs(extra or {}) do
    env[key] = value
  end
  return env
end

local function opts(name, extra_env)
  return {
    env = base_env(name, extra_env),
  }
end

local function mock_repo_env(value)
  t.mock_command('printf %s "$FKST_GITHUB_REPO"', { stdout = value or "owner/x" })
end

local function mock_write_env(value)
  t.mock_command('printf %s "$FKST_GITHUB_WRITE"', { stdout = value or "" })
end

local function mock_bot_env(value)
  t.mock_command('printf %s "$FKST_GITHUB_BOT_LOGIN"', { stdout = value or "fkst-test-bot" })
end

local function mock_issue_list(stdout, exit_code, stderr)
  t.mock_command("gh api --paginate --slurp 'repos/owner/x/issues?state=open&per_page=100'", {
    stdout = stdout or issue_list_json(),
    stderr = stderr or "",
    exit_code = exit_code or 0,
  })
end

local function mock_pr_list(stdout, exit_code, stderr)
  t.mock_command("gh api --paginate --slurp 'repos/owner/x/pulls?state=open&per_page=100'", {
    stdout = stdout or pr_list_json(),
    stderr = stderr or "",
    exit_code = exit_code or 0,
  })
end

local function mock_poll(issue_stdout, pr_stdout)
  mock_repo_env()
  mock_issue_list(issue_stdout)
  mock_pr_list(pr_stdout)
end

local function json_string(value)
  return tostring(value or "")
    :gsub("\\", "\\\\")
    :gsub('"', '\\"')
    :gsub("\n", "\\n")
end

local function comment_json(body, author, id, database_id)
  local id_field = ""
  if id ~= nil then
    id_field = '"id":"' .. json_string(id) .. '",'
  end
  local database_id_field = ""
  if database_id ~= nil then
    database_id_field = '"databaseId":' .. tostring(database_id) .. ","
  end
  return string.format('{%s%s"body":"%s","author":{"login":"%s"}}', id_field, database_id_field, json_string(body), json_string(author or "fkst-test-bot"))
end

local function mock_comment_view(comments, author)
  local rendered_comments = comments
  if type(comments) == "table" then
    local parts = {}
    for _, comment in ipairs(comments) do
      table.insert(parts, comment_json(comment.body, comment.author_login or comment.author, comment.id, comment.databaseId or comment.database_id))
    end
    rendered_comments = table.concat(parts, ",")
  else
    rendered_comments = comment_json(comments or "existing comment", author)
  end
  t.mock_command("gh issue view", {
    stdout = '{"comments":[' .. rendered_comments .. "]}\n",
  })
end

local function mock_comment_view_failure()
  t.mock_command("gh issue view", {
    stdout = "",
    stderr = "forced comment view failure",
    exit_code = 1,
  })
end

local function mock_label_view(labels)
  local parts = {}
  for _, label in ipairs(labels or {}) do
    table.insert(parts, string.format('{"name":"%s"}', label))
  end
  t.mock_command("gh issue view", {
    stdout = '{"labels":[' .. table.concat(parts, ",") .. "]}\n",
  })
end

local function mock_pr_open_guard(labels, comments)
  local rendered_labels = {}
  for _, label in ipairs(labels or { "fkst-dev:implementing" }) do
    table.insert(rendered_labels, string.format('{"name":"%s"}', json_string(label)))
  end
  local rendered_comments = {}
  for _, comment in ipairs(comments or {}) do
    if type(comment) == "table" then
      table.insert(rendered_comments, comment_json(comment.body, comment.author_login or comment.author))
    else
      table.insert(rendered_comments, comment_json(comment, "fkst-test-bot"))
    end
  end
  t.mock_command("--json labels,comments", {
    stdout = '{"labels":[' .. table.concat(rendered_labels, ",") .. '],"comments":[' .. table.concat(rendered_comments, ",") .. "]}\n",
    stderr = "",
    exit_code = 0,
  })
end

local function mock_branch_head(head_sha)
  t.mock_command("git show-ref --verify refs/heads", {
    stdout = tostring(head_sha or "abc123") .. " refs/heads/devloop-owner-x-42-01HY\n",
    stderr = "",
    exit_code = 0,
  })
end

local function mock_non_branch_ref_head(head_sha)
  t.mock_command("git show-ref --verify refs/heads", {
    stdout = tostring(head_sha or "abc123") .. " refs/tags/devloop-owner-x-42-01HY\n",
    stderr = "",
    exit_code = 0,
  })
end

local function mock_comment_write()
  t.mock_command("gh issue comment", { stdout = "", exit_code = 0 })
end

local function label_list_json(labels)
  local parts = {}
  for _, label in ipairs(labels or {}) do
    table.insert(parts, string.format('{"name":"%s"}', json_string(label)))
  end
  return "[" .. table.concat(parts, ",") .. "]\n"
end

local default_repo_labels = {
  "fkst-dev:enabled",
  "fkst-dev:thinking",
  "fkst-dev:ready",
  "fkst-dev:implementing",
  "fkst-dev:pr-open",
  "fkst-dev:reviewing",
  "fkst-dev:merge-ready",
  "fkst-dev:fixing",
  "fkst-dev:blocked",
  "fkst-dev:blocked-on-dependency",
  "fkst-dev:impl-failed",
}

local function mock_repo_label_list(labels)
  t.mock_command("gh label list", {
    stdout = label_list_json(labels or default_repo_labels),
    stderr = "",
    exit_code = 0,
  })
end

local function mock_label_create(exit_code, stderr)
  t.mock_command("gh label create", {
    stdout = "",
    stderr = stderr or "",
    exit_code = exit_code or 0,
  })
end

local function mock_label_write(labels)
  mock_repo_label_list(labels)
  t.mock_command("gh issue edit", { stdout = "", exit_code = 0 })
end

local function mock_pr_head_list(stdout)
  t.mock_command("gh api --paginate --slurp 'repos/owner/x/pulls?state=open&head=owner%3A", {
    stdout = stdout or "[]\n",
    stderr = "",
    exit_code = 0,
  })
end

local function mock_pr_head_state(head_sha, state, head_repo, is_cross_repository, base_branch)
  local cross = "false"
  if is_cross_repository == true then
    cross = "true"
  end
  t.mock_command("--json headRefOid", {
    stdout = string.format(
      '{"headRefOid":"%s","baseRefName":"%s","state":"%s","headRepository":{"nameWithOwner":"%s"},"isCrossRepository":%s}\n',
      head_sha or "abc123",
      base_branch or "dev",
      state or "OPEN",
      head_repo or "owner/x",
      cross
    ),
    stderr = "",
    exit_code = 0,
  })
end

local function mock_git_push()
  t.mock_command("git push -u origin", {
    stdout = "",
    stderr = "",
    exit_code = 0,
  })
end

local function mock_pr_create(number)
  t.mock_command("gh pr create", {
    stdout = string.format("https://github.example/owner/x/pull/%d\n", number or 7),
    stderr = "",
    exit_code = 0,
  })
end

local function mock_pr_create_stdout(stdout)
  t.mock_command("gh pr create", {
    stdout = stdout or "",
    stderr = "",
    exit_code = 0,
  })
end

local function mock_pr_comment_view(comments, author)
  local rendered_comments = comments
  if type(comments) == "table" then
    local parts = {}
    for _, comment in ipairs(comments) do
      table.insert(parts, comment_json(comment.body, comment.author_login or comment.author, comment.id, comment.databaseId or comment.database_id))
    end
    rendered_comments = table.concat(parts, ",")
  else
    rendered_comments = comment_json(comments or "existing pr comment", author)
  end
  t.mock_command("gh pr view", {
    stdout = '{"comments":[' .. rendered_comments .. "]}\n",
  })
end

local function mock_pr_comment_write()
  t.mock_command("gh pr comment", { stdout = "", exit_code = 0 })
end

local function calls_matching(needle)
  local matches = {}
  for _, call in ipairs(t.command_calls()) do
    if call.rendered:find(needle, 1, true) ~= nil then
      table.insert(matches, call)
    end
  end
  return matches
end

local function count_calls(needle)
  return #calls_matching(needle)
end

local function package_root()
  local source = package.searchpath("tests.proxy_integration_helpers", package.path)
  return source:match("(.+)/tests/proxy_integration_helpers%.lua$")
end

local function capture_comment_department_logs(department_path, event, write_env)
  mock_write_env(write_env)

  local captured = {}
  local old_log = log
  local old_write_comment_request = core.write_comment_request
  local write_requests = 0

  log = {
    info = function(message)
      table.insert(captured, tostring(message))
    end,
    warn = function(message)
      table.insert(captured, tostring(message))
    end,
    error = function(message)
      table.insert(captured, tostring(message))
    end,
  }
  core.write_comment_request = function(_payload, _target)
    write_requests = write_requests + 1
    core.read_env("FKST_GITHUB_WRITE")
  end

  local ok, err = pcall(function()
    dofile(package_root() .. "/" .. department_path)
    pipeline(event)
  end)

  core.write_comment_request = old_write_comment_request
  log = old_log
  if not ok then
    error(err)
  end

  return captured, write_requests
end

local function capture_label_department_logs(department_path, event, write_env, apply_result)
  mock_write_env(write_env)

  local captured = {}
  local old_log = log
  local old_apply_issue_labels = core.apply_issue_labels
  local old_with_lock = with_lock
  local write_requests = 0

  log = {
    info = function(message)
      table.insert(captured, tostring(message))
    end,
    warn = function(message)
      table.insert(captured, tostring(message))
    end,
    error = function(message)
      table.insert(captured, tostring(message))
    end,
  }
  core.apply_issue_labels = function(_repo, _issue_number, _add_labels, _remove_labels)
    write_requests = write_requests + 1
    if apply_result == false then
      return false
    end
    return true
  end
  with_lock = function(_key, fn)
    return fn()
  end

  local ok, err = pcall(function()
    dofile(package_root() .. "/" .. department_path)
    pipeline(event)
  end)

  core.apply_issue_labels = old_apply_issue_labels
  with_lock = old_with_lock
  log = old_log
  if not ok then
    error(err)
  end

  return captured, write_requests
end

local function long_dedup(suffix, total_len)
  local prefix = "github-devloop/issue/owner/x/42/result/"
  return prefix .. string.rep("v", total_len - #prefix - #suffix) .. suffix
end

local function pr_open_event()
  return {
    queue = "github_pr_open_request",
    payload = {
      schema = "github-proxy.pr-open.v1",
      repo = "owner/x",
      issue_number = 42,
      proposal_id = "github-devloop/issue/owner/x/42",
      impl_version = "v1",
      expected_state = "implementing",
      expected_version = "v1",
      branch = "devloop-owner-x-42-01HY",
      head_sha = "abc123",
      title = "Implement decision recorder",
      base_branch = "dev",
      body = 'github-devloop implementation PR for issue #42\n\n<!-- fkst:github-devloop:pr-origin:v1 proposal="github-devloop/issue/owner/x/42" issue="42" branch="devloop-owner-x-42-01HY" impl_version="v1" base_branch="dev" -->',
      issue_comment_body_template = 'github-devloop PR opened: #{{pr_number}}\n\n<!-- fkst:github-devloop:state:v1 proposal="github-devloop/issue/owner/x/42" state="pr-open" version="v1" stage_rank="650" -->\n<!-- fkst:github-devloop:pr-link:v1 proposal="github-devloop/issue/owner/x/42" pr="{{pr_number}}" branch="devloop-owner-x-42-01HY" impl_version="v1" base_branch="dev" -->',
      issue_label_add = { "fkst-dev:pr-open" },
      issue_label_remove = { "fkst-dev:implementing" },
      dedup_key = "open-pr/github-devloop/issue/owner/x/42/v1/devloop-owner-x-42-01HY",
      source_ref = {
        kind = "external",
        ref = "owner/x#issue/42",
      },
    },
  }
end

local function pr_open_guard_comments(extra)
  local comments = {
    '<!-- fkst:github-devloop:state:v1 proposal="github-devloop/issue/owner/x/42" state="implementing" version="v1" stage_rank="600" -->',
    '<!-- fkst:github-devloop:implementing:v1 proposal="github-devloop/issue/owner/x/42" dedup="v1" branch="devloop-owner-x-42-01HY" head_sha="abc123" base_branch="dev" base_sha="abc123" -->',
  }
  for _, comment in ipairs(extra or {}) do
    table.insert(comments, comment)
  end
  return comments
end

local function pr_open_visible_comments(extra)
  local comments = {
    'github-devloop PR opened: #9\n\n<!-- fkst:github-devloop:state:v1 proposal="github-devloop/issue/owner/x/42" state="pr-open" version="v1" stage_rank="650" -->\n<!-- fkst:github-devloop:pr-link:v1 proposal="github-devloop/issue/owner/x/42" pr="9" branch="devloop-owner-x-42-01HY" impl_version="v1" base_branch="dev" -->\n' .. core.comment_marker("open-pr/github-devloop/issue/owner/x/42/v1/devloop-owner-x-42-01HY"),
  }
  for _, comment in ipairs(extra or {}) do
    table.insert(comments, comment)
  end
  return pr_open_guard_comments(comments)
end

local function reviewing_marker()
  return '<!-- fkst:github-devloop:state:v1 proposal="github-devloop/issue/owner/x/42" state="reviewing" version="v1" stage_rank="675" -->'
end


return {
  t = t,
  core = core,
  issue_list_json = issue_list_json,
  pr_list_json = pr_list_json,
  runtime_root = runtime_root,
  opts = opts,
  mock_repo_env = mock_repo_env,
  mock_write_env = mock_write_env,
  mock_bot_env = mock_bot_env,
  mock_issue_list = mock_issue_list,
  mock_pr_list = mock_pr_list,
  mock_poll = mock_poll,
  json_string = json_string,
  comment_json = comment_json,
  mock_comment_view = mock_comment_view,
  mock_comment_view_failure = mock_comment_view_failure,
  mock_label_view = mock_label_view,
  mock_pr_open_guard = mock_pr_open_guard,
  mock_branch_head = mock_branch_head,
  mock_non_branch_ref_head = mock_non_branch_ref_head,
  mock_comment_write = mock_comment_write,
  mock_repo_label_list = mock_repo_label_list,
  mock_label_create = mock_label_create,
  mock_label_write = mock_label_write,
  mock_pr_head_list = mock_pr_head_list,
  mock_pr_head_state = mock_pr_head_state,
  mock_git_push = mock_git_push,
  mock_pr_create = mock_pr_create,
  mock_pr_create_stdout = mock_pr_create_stdout,
  mock_pr_comment_view = mock_pr_comment_view,
  mock_pr_comment_write = mock_pr_comment_write,
  calls_matching = calls_matching,
  count_calls = count_calls,
  capture_comment_department_logs = capture_comment_department_logs,
  capture_label_department_logs = capture_label_department_logs,
  long_dedup = long_dedup,
  pr_open_event = pr_open_event,
  pr_open_guard_comments = pr_open_guard_comments,
  pr_open_visible_comments = pr_open_visible_comments,
  reviewing_marker = reviewing_marker,
}
