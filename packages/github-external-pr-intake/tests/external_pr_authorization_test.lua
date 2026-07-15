local content_filter = require("forge.github.content_filter")
local core = require("core")
local strings = require("contract.strings")
local t = fkst.test

local function load_department()
  local old_pipeline = pipeline
  local module = require("departments.external_pr_intake.main")
  pipeline = old_pipeline
  return module
end

local function pr_json(model)
  model.read_count = model.read_count + 1
  local author = model.authors[math.min(model.read_count, #model.authors)]
  local assignees = model.claimed and '[{"login":"fkst-test-bot"}]' or "[]"
  return table.concat({
    '{"number":7,"title":',
    strings.json_string(model.title),
    ',"headRefName":"feature/contrib","baseRefName":"dev","state":"OPEN"',
    ',"createdAt":"2026-06-03T01:02:03Z","updatedAt":"2026-06-19T01:02:03Z"',
    ',"author":{"login":',
    strings.json_string(author),
    '},"comments":[],"assignees":',
    assignees,
    "}\n",
  })
end

local function fake_github(opts)
  local options = opts or {}
  local model = {
    allowed = content_filter.build_whitelist(options.allowed or {}),
    authors = options.authors or { "contributor" },
    claimed = false,
    read_count = 0,
    title = options.title or "Contributor patch",
    writes = {},
  }
  local handle = { _model = model }

  function handle.is_authorized_author(login)
    return content_filter.is_authorized(login, model.allowed)
  end

  function handle.pr_list(repo, timeout)
    table.insert(model.writes, { kind = "pr_list", repo = repo, timeout = timeout })
    return { stdout = "[" .. pr_json(model):gsub("%s+$", "") .. "]\n", stderr = "", exit_code = 0 }
  end

  function handle.pr_cli_view(repo, pr_number, fields, timeout)
    table.insert(model.writes, {
      kind = "pr_cli_view",
      repo = repo,
      pr_number = pr_number,
      fields = fields,
      timeout = timeout,
    })
    return { stdout = pr_json(model), stderr = "", exit_code = 0 }
  end

  function handle.issue_search(repo, query, fields, timeout)
    table.insert(model.writes, {
      kind = "issue_search",
      repo = repo,
      query = query,
      fields = fields,
      timeout = timeout,
    })
    return { stdout = "[]\n", stderr = "", exit_code = 0 }
  end

  function handle.issue_assign(repo, issue_number, login, timeout)
    model.claimed = true
    table.insert(model.writes, {
      kind = "issue_assign",
      repo = repo,
      issue_number = issue_number,
      login = login,
      timeout = timeout,
    })
    return { stdout = "", stderr = "", exit_code = 0 }
  end

  function handle.issue_create(repo, title, body_file, labels, assignees, timeout)
    table.insert(model.writes, {
      kind = "issue_create",
      repo = repo,
      title = title,
      body = file.read(body_file),
      labels = labels,
      assignees = assignees,
      timeout = timeout,
    })
    return { stdout = "https://github.com/owner/repo/issues/77\n", stderr = "", exit_code = 0 }
  end

  function handle.pr_comment(repo, pr_number, body_file, timeout)
    table.insert(model.writes, {
      kind = "pr_comment",
      repo = repo,
      pr_number = pr_number,
      body = file.read(body_file),
      timeout = timeout,
    })
    return { stdout = "", stderr = "", exit_code = 0 }
  end

  function handle.issue_close(repo, issue_number, timeout)
    table.insert(model.writes, {
      kind = "issue_close",
      repo = repo,
      issue_number = issue_number,
      timeout = timeout,
    })
    return { stdout = "", stderr = "", exit_code = 0 }
  end

  return handle
end

local function candidate_event()
  return {
    queue = "external_pr_candidate",
    payload = {
      schema = "github-external-pr-intake.v1",
      repo = "owner/repo",
      number = 7,
      dedup_key = "github-external-pr-intake/owner/repo/pr/7",
      source_ref = {
        kind = "external",
        ref = "owner/repo#pr/7",
      },
    },
  }
end

local function run_event(github, event)
  local files = {}
  local logs = {}
  local raises = {}
  local old_file = file
  local old_log = log
  local old_now = now
  local old_raise = raise
  local old_read_env = core.read_env
  local old_with_lock = with_lock

  file = {
    read = function(path)
      return files[path] or ""
    end,
    write = function(path, body)
      files[path] = body
    end,
  }
  log = {
    error = function(message)
      table.insert(logs, tostring(message))
    end,
    info = function(message)
      table.insert(logs, tostring(message))
    end,
    warn = function(message)
      table.insert(logs, tostring(message))
    end,
  }
  now = function()
    return 1780459324
  end
  raise = function(queue, payload)
    table.insert(raises, { queue = queue, payload = payload })
  end
  core.read_env = function(name)
    return ({
      FKST_GITHUB_BOT_LOGIN = "fkst-test-bot",
      FKST_GITHUB_REPO = "owner/repo",
      FKST_GITHUB_WRITE = "1",
      FKST_DEVLOOP_MANAGED_BOT_LOGINS = "fkst-test-bot,other-bot",
    })[name] or ""
  end
  with_lock = function(_key, fn)
    return fn()
  end

  local ok, err = pcall(function()
    load_department().make_department({ github = github }).pipeline(event)
  end)

  file = old_file
  log = old_log
  now = old_now
  raise = old_raise
  core.read_env = old_read_env
  with_lock = old_with_lock
  if not ok then
    error(err, 0)
  end
  return logs, raises
end

local function run_candidate(github)
  return run_event(github, candidate_event())
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

local function logs_contain(logs, needle)
  for _, message in ipairs(logs or {}) do
    if message:find(needle, 1, true) ~= nil then
      return true
    end
  end
  return false
end

return {
  test_trusted_contributor_env_is_allowed_for_action_scoped_policy_wiring = function()
    t.eq(
      core.read_env_command("FKST_EXTERNAL_PR_TRUSTED_CONTRIBUTOR_LOGINS"),
      'printf %s "$FKST_EXTERNAL_PR_TRUSTED_CONTRIBUTOR_LOGINS"'
    )
  end,

  test_non_authorized_durable_candidate_is_rejected_before_bridge_writes = function()
    local github = fake_github({ allowed = {}, authors = { "untrusted-contributor" } })
    local logs = run_candidate(github)

    t.eq(count_kind(github._model.writes, "pr_cli_view"), 1)
    t.eq(count_kind(github._model.writes, "issue_assign"), 0)
    t.eq(count_kind(github._model.writes, "issue_create"), 0)
    t.eq(count_kind(github._model.writes, "pr_comment"), 0)
    t.is_true(logs_contain(logs, "action=skip-non-authorized-author"))
  end,

  test_scan_does_not_admit_non_authorized_author = function()
    local github = fake_github({ allowed = {}, authors = { "untrusted-contributor" } })
    local logs, raises = run_event(github, {
      queue = "external_pr_scan",
      payload = { schema = "github-external-pr-intake.v1" },
    })

    t.eq(#raises, 0)
    t.eq(count_kind(github._model.writes, "pr_list"), 1)
    t.eq(count_kind(github._model.writes, "pr_cli_view"), 0)
    t.is_true(logs_contain(logs, "action=skip-non-authorized-author"))
  end,

  test_authorization_is_rechecked_after_claim_before_bridge_creation = function()
    local github = fake_github({
      allowed = { "trusted-contributor" },
      authors = { "trusted-contributor", "authorization-revoked" },
    })
    local logs = run_candidate(github)

    t.eq(count_kind(github._model.writes, "pr_cli_view"), 2)
    t.eq(count_kind(github._model.writes, "issue_assign"), 1)
    t.eq(count_kind(github._model.writes, "issue_create"), 0)
    t.is_true(logs_contain(logs, "action=skip-non-authorized-author-after-claim"))
  end,

  test_explicitly_trusted_contributor_gets_metadata_only_bridge_identity = function()
    local github = fake_github({
      allowed = { "trusted-contributor" },
      authors = { "trusted-contributor" },
      title = content_filter.redaction_marker("title", "trusted-contributor"),
    })
    run_candidate(github)

    local created = write_of_kind(github._model.writes, "issue_create")
    t.eq(count_kind(github._model.writes, "issue_create"), 1)
    t.eq(created.title, "Integrate external PR #7 from @trusted-contributor")
    t.is_true(created.title:find(content_filter.MARKER_PREFIX, 1, true) == nil)
  end,

  test_bridge_issue_title_never_uses_redacted_pr_prose = function()
    local title = core.bridge_issue_title({
      number = 7,
      author_login = "trusted-contributor",
      title = content_filter.redaction_marker("title", "trusted-contributor"),
    })

    t.eq(title, "Integrate external PR #7 from @trusted-contributor")
    t.is_true(title:find(content_filter.MARKER_PREFIX, 1, true) == nil)
  end,

  test_bridge_issue_title_rejects_marker_bearing_author_identity = function()
    local ok, err = pcall(core.bridge_issue_title, {
      number = 7,
      author_login = content_filter.redaction_marker("author", "untrusted-contributor"),
      title = "ignored prose",
    })

    t.eq(ok, false)
    t.is_true(tostring(err):find("bridge-title-marker-forbidden", 1, true) ~= nil)
  end,
}
