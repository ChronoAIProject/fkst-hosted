local t = fkst.test

local function package_root()
  local source = package.searchpath("tests.external_pr_intake_test", package.path)
  return source:match("(.+)/tests/external_pr_intake_test%.lua$")
end

local function load_department()
  local old_pipeline = pipeline
  local module = dofile(package_root() .. "/departments/external_pr_intake/main.lua")
  pipeline = old_pipeline
  return module
end

local function json_string(value)
  return require("std.strings").json_string(value)
end

local function pr_json(pr)
  local comments = {}
  for _, comment in ipairs(pr.comments or {}) do
    table.insert(comments, '{"body":' .. json_string(comment.body or "")
      .. ',"author":{"login":' .. json_string(comment.author_login or "fkst-test-bot") .. "}}")
  end
  local assignees = {}
  for _, login in ipairs(pr.assignees or {}) do
    table.insert(assignees, '{"login":' .. json_string(login) .. "}")
  end
  return '{"number":' .. tostring(pr.number or 7)
    .. ',"title":' .. json_string(pr.title or "Contributor patch")
    .. ',"headRefName":' .. json_string(pr.head_ref_name or "feature/contrib")
    .. ',"baseRefName":' .. json_string(pr.base_ref_name or "dev")
    .. ',"state":' .. json_string(pr.state or "OPEN")
    .. ',"updatedAt":' .. json_string(pr.updated_at or "2026-06-19T01:02:03Z")
    .. ',"author":{"login":' .. json_string(pr.author_login or "contributor")
    .. '},"comments":[' .. table.concat(comments, ",")
    .. '],"assignees":[' .. table.concat(assignees, ",") .. "]}\n"
end

local function pr_list_json(prs)
  local parts = {}
  for _, pr in pairs(prs or {}) do
    table.insert(parts, (pr_json(pr):gsub("%s+$", "")))
  end
  return "[" .. table.concat(parts, ",") .. "]\n"
end

local function issue_json(issue)
  return '{"number":' .. tostring(issue.number or 77)
    .. ',"title":' .. json_string(issue.title or "Bridge")
    .. ',"state":' .. json_string(issue.state or "OPEN")
    .. ',"author":{"login":' .. json_string(issue.author_login or "fkst-test-bot")
    .. '},"body":' .. json_string(issue.body or "") .. "}"
end

local function new_fake_github(opts)
  local options = opts or {}
  local model = {
    writes = {},
    prs = options.prs or {
      [7] = {
        number = 7,
        title = "Contributor patch",
        author_login = "contributor",
        head_ref_name = "feature/contrib",
        base_ref_name = "dev",
        state = "OPEN",
        comments = {},
        assignees = {},
      },
    },
    list = options.list,
    issues = options.issues or {},
    next_issue = options.next_issue or 77,
    hidden_issues_until_create = options.hidden_issues_until_create == true,
    created_count = 0,
  }
  local handle = { _model = model }
  function handle.pr_list(repo, timeout)
    table.insert(model.writes, { kind = "pr_list", repo = repo, timeout = timeout })
    return { stdout = pr_list_json(model.list or model.prs), stderr = "", exit_code = 0 }
  end
  function handle.pr_cli_view(repo, pr_number, fields, timeout)
    table.insert(model.writes, { kind = "pr_cli_view", repo = repo, pr_number = pr_number, fields = fields, timeout = timeout })
    local pr = model.prs[pr_number]
    if pr == nil then
      error("fake: unknown PR " .. tostring(pr_number))
    end
    return { stdout = pr_json(pr), stderr = "", exit_code = 0 }
  end
  function handle.issue_search(repo, query, fields, timeout)
    table.insert(model.writes, { kind = "issue_search", repo = repo, query = query, fields = fields, timeout = timeout })
    local parts = {}
    if not model.hidden_issues_until_create or model.created_count > 0 then
      for _, issue in ipairs(model.issues or {}) do
        if tostring(issue.body or ""):find(query, 1, true) ~= nil then
          table.insert(parts, issue_json(issue))
        end
      end
    end
    return { stdout = "[" .. table.concat(parts, ",") .. "]\n", stderr = "", exit_code = 0 }
  end
  function handle.issue_assign(repo, issue_number, login, timeout)
    table.insert(model.writes, { kind = "issue_assign", repo = repo, issue_number = issue_number, login = login, timeout = timeout })
    local pr = model.prs[issue_number]
    pr.assignees = pr.assignees or {}
    local present = false
    for _, assignee in ipairs(pr.assignees) do
      if assignee == login then
        present = true
      end
    end
    if not present then
      table.insert(pr.assignees, login)
    end
    return { stdout = "", stderr = "", exit_code = 0 }
  end
  function handle.issue_create(repo, title, body_file, labels, assignees, timeout)
    local body = file.read(body_file)
    table.insert(model.writes, { kind = "issue_create", repo = repo, title = title, body = body, labels = labels, assignees = assignees, timeout = timeout })
    model.created_count = model.created_count + 1
    table.insert(model.issues, {
      number = model.next_issue,
      author_login = "fkst-test-bot",
      state = "OPEN",
      body = body,
    })
    return { stdout = "https://github.com/" .. tostring(repo) .. "/issues/" .. tostring(model.next_issue) .. "\n", stderr = "", exit_code = 0 }
  end
  function handle.pr_comment(repo, pr_number, body_file, timeout)
    local body = file.read(body_file)
    table.insert(model.writes, { kind = "pr_comment", repo = repo, pr_number = pr_number, body = body, timeout = timeout })
    local pr = model.prs[pr_number]
    pr.comments = pr.comments or {}
    table.insert(pr.comments, { author_login = "fkst-test-bot", body = body })
    return { stdout = "", stderr = "", exit_code = 0 }
  end
  function handle.issue_close(repo, issue_number, timeout)
    table.insert(model.writes, { kind = "issue_close", repo = repo, issue_number = issue_number, timeout = timeout })
    for _, issue in ipairs(model.issues or {}) do
      if tonumber(issue.number) == tonumber(issue_number) then
        issue.state = "CLOSED"
      end
    end
    return { stdout = "", stderr = "", exit_code = 0 }
  end
  return handle
end

local function run_pipeline(opts)
  local options = opts or {}
  local github = options.github or new_fake_github(options.github_opts)
  local files = {}
  local raises = {}
  local locks = {}
  local old_file = file
  local old_log = log
  local old_raise = raise
  local old_with_lock = with_lock
  file = {
    write = function(path, body)
      files[path] = body
    end,
    read = function(path)
      return files[path] or ""
    end,
  }
  log = {
    info = function(_message) end,
    warn = function(_message) end,
    error = function(_message) end,
  }
  raise = function(queue, payload)
    table.insert(raises, { queue = queue, payload = payload })
  end
  with_lock = function(key, fn)
    table.insert(locks, key)
    return fn()
  end

  local module = load_department()
  local dept = module.make_department({ github = github })
  local core = require("core")
  local old_read = core.read_env
  local env = options.env or {
    FKST_GITHUB_REPO = "owner/repo",
    FKST_GITHUB_WRITE = "1",
    FKST_GITHUB_BOT_LOGIN = "fkst-test-bot",
    FKST_DEVLOOP_MANAGED_BOT_LOGINS = "fkst-test-bot,other-bot",
  }
  core.read_env = function(name)
    return env[name] or ""
  end
  local ok, err = pcall(function()
    dept.pipeline(options.event)
  end)
  core.read_env = old_read
  file = old_file
  log = old_log
  raise = old_raise
  with_lock = old_with_lock
  if not ok then
    error(err, 0)
  end
  return { github = github, files = files, raises = raises, locks = locks }
end

local function count_kind(writes, kind)
  local count = 0
  for _, write in ipairs(writes or {}) do
    if write.kind == kind then
      count = count + 1
    end
  end
  return count
end

local function write_of_kind(writes, kind, ordinal)
  local seen = 0
  for _, write in ipairs(writes or {}) do
    if write.kind == kind then
      seen = seen + 1
      if seen == (ordinal or 1) then
        return write
      end
    end
  end
  return nil
end

local function candidate_event(number)
  number = number or 7
  return {
    queue = "external_pr_candidate",
    payload = {
      schema = "github-external-pr-intake.v1",
      repo = "owner/repo",
      number = number,
      dedup_key = "github-external-pr-intake/owner/repo/pr/" .. tostring(number),
      source_ref = {
        kind = "external",
        ref = "owner/repo#pr/" .. tostring(number),
      },
    },
  }
end

return {
  test_scan_raises_only_external_candidates = function()
    local github = new_fake_github({
      list = {
        {
          number = 7,
          title = "Contributor patch",
          author_login = "contributor",
          head_ref_name = "feature/contrib",
          state = "OPEN",
        },
        {
          number = 8,
          title = "Bot patch",
          author_login = "fkst-test-bot[bot]",
          head_ref_name = "feature/bot",
          state = "OPEN",
        },
        {
          number = 9,
          title = "Managed branch",
          author_login = "contributor",
          head_ref_name = "devloop/owner-repo-9",
          state = "OPEN",
        },
        {
          number = 10,
          title = "Closed patch",
          author_login = "contributor",
          head_ref_name = "feature/closed",
          state = "CLOSED",
        },
      },
    })
    local result = run_pipeline({
      github = github,
      event = { queue = "external_pr_scan", payload = { schema = "github-external-pr-intake.v1" } },
    })

    t.eq(#result.raises, 1)
    t.eq(result.raises[1].queue, "external_pr_candidate")
    t.eq(result.raises[1].payload.repo, "owner/repo")
    t.eq(result.raises[1].payload.number, 7)
    t.eq(result.raises[1].payload.source_ref.kind, "external")
    t.eq(result.raises[1].payload.source_ref.ref, "owner/repo#pr/7")
    t.eq(count_kind(github._model.writes, "pr_list"), 1)
  end,

  test_candidate_creates_one_bridge_issue_and_pr_marker = function()
    local result = run_pipeline({
      event = candidate_event(7),
    })
    local writes = result.github._model.writes
    local created = write_of_kind(writes, "issue_create")
    local marker = write_of_kind(writes, "pr_comment")

    t.eq(count_kind(writes, "issue_assign"), 1)
    t.eq(count_kind(writes, "issue_create"), 1)
    t.eq(count_kind(writes, "pr_comment"), 1)
    t.eq(count_kind(writes, "issue_close"), 0)
    t.eq(created.title, "Integrate external PR #7: Contributor patch")
    t.eq(#created.labels, 0)
    t.is_true(created.body:find("source_ref: external:owner/repo#pr/7", 1, true) ~= nil)
    t.is_true(created.body:find("fetch `refs/pull/7/head`", 1, true) ~= nil)
    t.is_true(created.body:find("implement against `dev`", 1, true) ~= nil)
    t.is_true(marker.body:find('external-pr-bridge:v1 repo="owner/repo" pr="7"', 1, true) ~= nil)
    t.is_true(marker.body:find('issue="77"', 1, true) ~= nil)
    t.eq(#result.locks, 1)
  end,

  test_second_scan_dedups_on_trusted_pr_marker = function()
    local github = new_fake_github()
    run_pipeline({
      github = github,
      event = candidate_event(7),
    })
    run_pipeline({
      github = github,
      event = candidate_event(7),
    })

    t.eq(count_kind(github._model.writes, "issue_create"), 1)
    t.eq(count_kind(github._model.writes, "pr_comment"), 1)
  end,

  test_created_duplicate_bridge_is_reconciled_to_lowest_issue = function()
    local core = require("core")
    local github = new_fake_github({
      next_issue = 99,
      hidden_issues_until_create = true,
      issues = {
        {
          number = 88,
          author_login = "fkst-test-bot",
          state = "OPEN",
          body = core.bridge_marker("owner/repo", 7),
        },
      },
    })
    local result = run_pipeline({
      github = github,
      event = candidate_event(7),
    })
    local writes = result.github._model.writes
    local marker = write_of_kind(writes, "pr_comment")

    t.eq(count_kind(writes, "issue_create"), 1)
    t.eq(count_kind(writes, "issue_close"), 1)
    t.eq(write_of_kind(writes, "issue_close").issue_number, 99)
    t.is_true(marker.body:find('issue="88"', 1, true) ~= nil)
  end,

  test_existing_bridge_issue_search_dedups_without_pr_write = function()
    local core = require("core")
    local github = new_fake_github({
      issues = {
        {
          number = 88,
          author_login = "fkst-test-bot",
          state = "OPEN",
          body = core.bridge_marker("owner/repo", 7),
        },
      },
    })
    run_pipeline({
      github = github,
      event = candidate_event(7),
    })

    t.eq(count_kind(github._model.writes, "issue_create"), 0)
    t.eq(count_kind(github._model.writes, "pr_comment"), 0)
    t.eq(count_kind(github._model.writes, "issue_assign"), 0)
    t.eq(count_kind(github._model.writes, "issue_search"), 1)
  end,

  test_bot_authored_pr_is_ignored = function()
    local github = new_fake_github({
      prs = {
        [7] = {
          number = 7,
          title = "Bot patch",
          author_login = "other-bot[bot]",
          head_ref_name = "feature/bot",
          state = "OPEN",
          comments = {},
          assignees = {},
        },
      },
    })
    run_pipeline({
      github = github,
      event = candidate_event(7),
    })

    t.eq(count_kind(github._model.writes, "issue_create"), 0)
    t.eq(count_kind(github._model.writes, "issue_assign"), 0)
    t.eq(count_kind(github._model.writes, "issue_search"), 0)
  end,

  test_devloop_head_pr_is_ignored = function()
    local github = new_fake_github({
      prs = {
        [7] = {
          number = 7,
          title = "Managed branch",
          author_login = "contributor",
          head_ref_name = "devloop/owner-repo-7",
          state = "OPEN",
          comments = {},
          assignees = {},
        },
      },
    })
    run_pipeline({
      github = github,
      event = candidate_event(7),
    })

    t.eq(count_kind(github._model.writes, "issue_create"), 0)
    t.eq(count_kind(github._model.writes, "issue_assign"), 0)
    t.eq(count_kind(github._model.writes, "issue_search"), 0)
  end,

  test_other_assignee_claim_blocks_writes = function()
    local github = new_fake_github({
      prs = {
        [7] = {
          number = 7,
          title = "Contributor patch",
          author_login = "contributor",
          head_ref_name = "feature/contrib",
          state = "OPEN",
          comments = {},
          assignees = { "other-bot" },
        },
      },
    })
    run_pipeline({
      github = github,
      event = candidate_event(7),
    })

    t.eq(count_kind(github._model.writes, "issue_create"), 0)
    t.eq(count_kind(github._model.writes, "pr_comment"), 0)
    t.eq(count_kind(github._model.writes, "issue_assign"), 0)
  end,

  test_dry_run_does_not_claim_or_write = function()
    local github = new_fake_github()
    run_pipeline({
      github = github,
      env = {
        FKST_GITHUB_REPO = "owner/repo",
        FKST_GITHUB_WRITE = "",
        FKST_GITHUB_BOT_LOGIN = "fkst-test-bot",
        FKST_DEVLOOP_MANAGED_BOT_LOGINS = "fkst-test-bot",
      },
      event = candidate_event(7),
    })

    t.eq(count_kind(github._model.writes, "issue_create"), 0)
    t.eq(count_kind(github._model.writes, "pr_comment"), 0)
    t.eq(count_kind(github._model.writes, "issue_assign"), 0)
    t.eq(count_kind(github._model.writes, "issue_search"), 1)
  end,
}
