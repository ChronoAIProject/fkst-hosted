local strings = require("contract.strings")
local t = fkst.test

local function load_department()
  local old_pipeline = pipeline
  local module = require("departments.external_pr_intake.main")
  pipeline = old_pipeline
  return module
end

local function json_string(value)
  return strings.json_string(value)
end

local function render_comments(comments)
  local parts = {}
  for _, comment in ipairs(comments or {}) do
    table.insert(parts, '{"body":' .. json_string(comment.body or "")
      .. ',"author":{"login":' .. json_string(comment.author_login or "fkst-test-bot")
      .. '},"createdAt":' .. json_string(comment.created_at or "2026-06-03T01:02:03Z") .. "}")
  end
  return "[" .. table.concat(parts, ",") .. "]"
end

local function render_labels(labels)
  local parts = {}
  for _, label in ipairs(labels or {}) do
    table.insert(parts, '{"name":' .. json_string(label) .. "}")
  end
  return "[" .. table.concat(parts, ",") .. "]"
end

local function pr_json(pr)
  return '{"number":' .. tostring(pr.number or 7)
    .. ',"title":' .. json_string(pr.title or "Contributor patch")
    .. ',"headRefName":' .. json_string(pr.head_ref_name or "feature/contrib")
    .. ',"baseRefName":' .. json_string(pr.base_ref_name or "dev")
    .. ',"state":' .. json_string(pr.state or "OPEN")
    .. ',"createdAt":' .. json_string(pr.created_at or "2026-06-03T01:02:03Z")
    .. ',"updatedAt":' .. json_string(pr.updated_at or "2026-06-19T01:02:03Z")
    .. ',"author":{"login":' .. json_string(pr.author_login or "contributor")
    .. '},"comments":' .. render_comments(pr.comments)
    .. ',"assignees":[]}\n'
end

local function issue_json(issue)
  return '{"number":' .. tostring(issue.number or 77)
    .. ',"title":' .. json_string(issue.title or "Integrate external PR #7")
    .. ',"state":' .. json_string(issue.state or "CLOSED")
    .. ',"url":' .. json_string(issue.url or "https://github.com/owner/repo/issues/" .. tostring(issue.number or 77))
    .. ',"labels":' .. render_labels(issue.labels)
    .. ',"comments":' .. render_comments(issue.comments)
    .. ',"author":{"login":' .. json_string(issue.author_login or "fkst-test-bot")
    .. '},"body":' .. json_string(issue.body or "") .. "}"
end

local function new_fake_github(model)
  local handle = { _model = model }
  function handle.pr_list(repo, timeout)
    table.insert(model.writes, { kind = "pr_list", repo = repo, timeout = timeout })
    local parts = {}
    for _, pr in pairs(model.prs or {}) do
      if tostring(pr.state or "OPEN"):upper() == "OPEN" then
        table.insert(parts, (pr_json(pr):gsub("%s+$", "")))
      end
    end
    return { stdout = "[" .. table.concat(parts, ",") .. "]\n", stderr = "", exit_code = 0 }
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
    for _, issue in ipairs(model.issues or {}) do
      if tostring(issue.body or ""):find(query, 1, true) ~= nil then
        table.insert(parts, issue_json(issue))
      end
    end
    return { stdout = "[" .. table.concat(parts, ",") .. "]\n", stderr = "", exit_code = 0 }
  end
  function handle.issue_view(repo, issue_number, fields, timeout)
    table.insert(model.writes, { kind = "issue_view", repo = repo, issue_number = issue_number, fields = fields, timeout = timeout })
    for _, issue in ipairs(model.issues or {}) do
      if tonumber(issue.number) == tonumber(issue_number) then
        return { stdout = issue_json(issue), stderr = "", exit_code = 0 }
      end
    end
    error("fake: unknown issue " .. tostring(issue_number))
  end
  function handle.pr_comment(repo, pr_number, body_file, timeout)
    local body = file.read(body_file)
    table.insert(model.writes, { kind = "pr_comment", repo = repo, pr_number = pr_number, body = body, timeout = timeout })
    local pr = model.prs[pr_number]
    pr.comments = pr.comments or {}
    table.insert(pr.comments, { author_login = "fkst-test-bot", body = body })
    return { stdout = "", stderr = "", exit_code = 0 }
  end
  function handle.pr_close(repo, pr_number, timeout)
    table.insert(model.writes, { kind = "pr_close", repo = repo, pr_number = pr_number, timeout = timeout })
    model.prs[pr_number].state = "CLOSED"
    return { stdout = "", stderr = "", exit_code = 0 }
  end
  return handle
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

local function write_of_kind(writes, kind)
  for _, write in ipairs(writes or {}) do
    if write.kind == kind then
      return write
    end
  end
  return nil
end

local function bridge_issue(fields)
  local core = require("core")
  fields = fields or {}
  fields.number = fields.number or 77
  fields.body = fields.body or core.bridge_marker("owner/repo", 7)
  return fields
end

local function merged_marker(issue_number, pr_number)
  return '<!-- fkst:github-devloop:merged:v1 proposal="github-devloop/issue/owner/repo/'
    .. tostring(issue_number)
    .. '" pr="'
    .. tostring(pr_number)
    .. '" version="v1" head_sha="0123456789abcdef0123456789abcdef01234567" -->'
end

local function merged_state_marker(issue_number)
  return '<!-- fkst:github-devloop:state:v1 proposal="github-devloop/issue/owner/repo/'
    .. tostring(issue_number)
    .. '" state="merged" version="v1" stage_rank="900" -->'
end

local function run_scan(options)
  local opts = options or {}
  local files = {}
  local raises = {}
  local locks = {}
  local old_file = file
  local old_log = log
  local old_raise = raise
  local old_with_lock = with_lock
  local old_now = now
  local core = require("core")
  local old_read = core.read_env
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
  now = function()
    return 1780459324
  end
  core.read_env = function(name)
    return (opts.env or {
      FKST_GITHUB_REPO = "owner/repo",
      FKST_GITHUB_WRITE = "1",
      FKST_GITHUB_BOT_LOGIN = "fkst-test-bot",
      FKST_DEVLOOP_MANAGED_BOT_LOGINS = "fkst-test-bot,other-bot",
    })[name] or ""
  end
  local model = opts.model or {
    writes = {},
    prs = {
      [7] = {
        number = 7,
        title = "Contributor patch",
        author_login = "contributor",
        head_ref_name = "feature/contrib",
        state = "OPEN",
        comments = {},
      },
    },
    issues = {},
  }
  local github = new_fake_github(model)
  local ok, err = pcall(function()
    local dept = load_department().make_department({ github = github })
    dept.pipeline({ queue = "external_pr_scan", payload = { schema = "github-external-pr-intake.v1" } })
  end)
  core.read_env = old_read
  now = old_now
  with_lock = old_with_lock
  raise = old_raise
  log = old_log
  file = old_file
  if not ok then
    error(err, 0)
  end
  return { github = github, raises = raises, locks = locks, files = files }
end

return {
  test_scan_acknowledges_and_closes_merged_bridge_issue = function()
    local model = {
      writes = {},
      prs = {
        [7] = { number = 7, author_login = "contributor", head_ref_name = "feature/contrib", state = "OPEN", comments = {} },
      },
      issues = {
        bridge_issue({
          labels = { "fkst-dev:enabled", "fkst-dev:merged" },
          comments = {
            { author_login = "fkst-test-bot", body = merged_state_marker(77) },
          },
        }),
      },
    }
    local result = run_scan({ model = model })
    local comment = write_of_kind(model.writes, "pr_comment")

    t.eq(#result.raises, 0)
    t.eq(count_kind(model.writes, "pr_comment"), 1)
    t.eq(count_kind(model.writes, "pr_close"), 1)
    t.is_true(comment.body:find("Thanks for the contribution.", 1, true) ~= nil)
    t.is_true(comment.body:find("Issue: https://github.com/owner/repo/issues/77", 1, true) ~= nil)
    t.is_true(comment.body:find('external-pr-handled:v1 repo="owner/repo" pr="7" issue="77"', 1, true) ~= nil)
    t.eq(model.prs[7].state, "CLOSED")
  end,

  test_scan_ignores_untrusted_merged_bridge_issue_marker = function()
    local model = {
      writes = {},
      prs = {
        [7] = { number = 7, author_login = "contributor", head_ref_name = "feature/contrib", state = "OPEN", comments = {} },
      },
      issues = {
        bridge_issue({
          labels = { "fkst-dev:enabled", "fkst-dev:merged" },
          comments = {
            { author_login = "contributor", body = merged_state_marker(77) },
          },
        }),
      },
    }
    local result = run_scan({ model = model })

    t.eq(count_kind(model.writes, "pr_comment"), 0)
    t.eq(count_kind(model.writes, "pr_close"), 0)
    t.eq(#result.raises, 1)
    t.eq(model.prs[7].state, "OPEN")
  end,

  test_scan_links_internal_pr_from_trusted_merged_marker = function()
    local model = {
      writes = {},
      prs = {
        [7] = { number = 7, author_login = "contributor", head_ref_name = "feature/contrib", state = "OPEN", comments = {} },
      },
      issues = {
        bridge_issue({
          comments = {
            { author_login = "fkst-test-bot", body = merged_marker(77, 88) },
          },
        }),
      },
    }
    run_scan({ model = model })
    local comment = write_of_kind(model.writes, "pr_comment")

    t.eq(count_kind(model.writes, "pr_comment"), 1)
    t.eq(count_kind(model.writes, "pr_close"), 1)
    t.is_true(comment.body:find("PR: https://github.com/owner/repo/pull/88", 1, true) ~= nil)
  end,

  test_scan_does_not_close_declined_bridge_issue = function()
    local model = {
      writes = {},
      prs = {
        [7] = { number = 7, author_login = "contributor", head_ref_name = "feature/contrib", state = "OPEN", comments = {} },
      },
      issues = {
        bridge_issue({ labels = { "fkst-dev:enabled", "fkst-dev:blocked" } }),
      },
    }
    local result = run_scan({ model = model })

    t.eq(count_kind(model.writes, "pr_comment"), 0)
    t.eq(count_kind(model.writes, "pr_close"), 0)
    t.eq(#result.raises, 1)
  end,

  test_scan_dry_run_logs_without_comment_or_close = function()
    local model = {
      writes = {},
      prs = {
        [7] = { number = 7, author_login = "contributor", head_ref_name = "feature/contrib", state = "OPEN", comments = {} },
      },
      issues = {
        bridge_issue({
          labels = { "fkst-dev:merged" },
          comments = {
            { author_login = "fkst-test-bot", body = merged_state_marker(77) },
          },
        }),
      },
    }
    local result = run_scan({
      model = model,
      env = {
        FKST_GITHUB_REPO = "owner/repo",
        FKST_GITHUB_WRITE = "",
        FKST_GITHUB_BOT_LOGIN = "fkst-test-bot",
        FKST_DEVLOOP_MANAGED_BOT_LOGINS = "fkst-test-bot",
      },
    })

    t.eq(count_kind(model.writes, "pr_comment"), 0)
    t.eq(count_kind(model.writes, "pr_close"), 0)
    t.eq(#result.raises, 0)
  end,

  test_existing_handled_marker_closes_without_double_comment = function()
    local core = require("core")
    local model = {
      writes = {},
      prs = {
        [7] = {
          number = 7,
          author_login = "contributor",
          head_ref_name = "feature/contrib",
          state = "OPEN",
          comments = {
            { author_login = "fkst-test-bot", body = core.handled_marker("owner/repo", 7, 77) },
          },
        },
      },
      issues = {
        bridge_issue({
          labels = { "fkst-dev:merged" },
          comments = {
            { author_login = "fkst-test-bot", body = merged_state_marker(77) },
          },
        }),
      },
    }
    run_scan({ model = model })

    t.eq(count_kind(model.writes, "pr_comment"), 0)
    t.eq(count_kind(model.writes, "pr_close"), 1)
  end,
}
