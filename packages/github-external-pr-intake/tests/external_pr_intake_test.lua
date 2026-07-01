local strings = require("contract.strings")
local t = fkst.test

local package_root = "packages/github-external-pr-intake"

local function load_department()
  local old_pipeline = pipeline
  local module = require("departments.external_pr_intake.main")
  pipeline = old_pipeline
  return module
end

local function json_string(value)
  return strings.json_string(value)
end

local function shell_quote(value)
  return "'" .. tostring(value):gsub("'", "'\\''") .. "'"
end

local function mkdir_p(path)
  local ok = os.execute("mkdir -p " .. shell_quote(path))
  if not (ok == true or ok == 0) then
    error("github-external-pr-intake: mkdir failed for " .. tostring(path))
  end
end

local function parent_dir(path)
  return tostring(path):match("^(.*)/[^/]+$") or "."
end

local function sibling_package_root(name)
  return parent_dir(package_root) .. "/" .. tostring(name)
end

local function write_disk_file(path, body)
  local handle = assert(io.open(path, "w"))
  handle:write(body)
  handle:close()
end

local function read_disk_file(path)
  local handle = assert(io.open(path, "r"))
  local body = handle:read("*a")
  handle:close()
  return body
end

local function wait_for_file(path, attempts)
  for _ = 1, attempts or 100 do
    if file.exists(path) then
      return true
    end
    os.execute("sleep 0.02")
  end
  return false
end

local function start_python_background(script, args)
  local parts = { "python3", shell_quote(script) }
  for _, arg in ipairs(args or {}) do
    table.insert(parts, shell_quote(arg))
  end
  local ok = os.execute(table.concat(parts, " ") .. " &")
  if not (ok == true or ok == 0) then
    error("github-external-pr-intake: failed to start python helper")
  end
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
    .. ',"createdAt":' .. json_string(pr.created_at or "2026-06-03T01:02:03Z")
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
    .. ',"url":' .. json_string(issue.url or "https://github.com/owner/repo/issues/" .. tostring(issue.number or 77))
    .. ',"labels":[' .. table.concat((function()
      local labels = {}
      for _, label in ipairs(issue.labels or {}) do
        table.insert(labels, '{"name":' .. json_string(label) .. "}")
      end
      return labels
    end)(), ",") .. "]"
    .. ',"comments":[' .. table.concat((function()
      local comments = {}
      for _, comment in ipairs(issue.comments or {}) do
        table.insert(comments, '{"body":' .. json_string(comment.body or "")
          .. ',"author":{"login":' .. json_string(comment.author_login or "fkst-test-bot") .. "}}")
      end
      return comments
    end)(), ",") .. "]"
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
    hidden_issues_until_creates = options.hidden_issues_until_creates or 0,
    hidden_comments_until_creates = options.hidden_comments_until_creates or 0,
    issue_create_yield = options.issue_create_yield,
    fail_pr_cli_view_once = options.fail_pr_cli_view_once,
    created_count = 0,
  }
  local handle = { _model = model }
  function handle.pr_list(repo, timeout)
    table.insert(model.writes, { kind = "pr_list", repo = repo, timeout = timeout })
    return { stdout = pr_list_json(model.list or model.prs), stderr = "", exit_code = 0 }
  end
  function handle.pr_cli_view(repo, pr_number, fields, timeout)
    table.insert(model.writes, { kind = "pr_cli_view", repo = repo, pr_number = pr_number, fields = fields, timeout = timeout })
    if model.fail_pr_cli_view_once then
      model.fail_pr_cli_view_once = false
      error("fake: transient PR view failure")
    end
    local pr = model.prs[pr_number]
    if pr == nil then
      error("fake: unknown PR " .. tostring(pr_number))
    end
    if model.created_count < model.hidden_comments_until_creates then
      local hidden = {}
      for key, value in pairs(pr) do
        if key ~= "comments" then
          hidden[key] = value
        end
      end
      hidden.comments = {}
      return { stdout = pr_json(hidden), stderr = "", exit_code = 0 }
    end
    return { stdout = pr_json(pr), stderr = "", exit_code = 0 }
  end
  function handle.issue_search(repo, query, fields, timeout)
    table.insert(model.writes, { kind = "issue_search", repo = repo, query = query, fields = fields, timeout = timeout })
    local parts = {}
    if (not model.hidden_issues_until_create or model.created_count > 0)
      and model.created_count >= model.hidden_issues_until_creates then
      for _, issue in ipairs(model.issues or {}) do
        if tostring(issue.body or ""):find(query, 1, true) ~= nil then
          table.insert(parts, issue_json(issue))
        end
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
    local issue_number = model.next_issue
    table.insert(model.issues, {
      number = issue_number,
      author_login = "fkst-test-bot",
      state = "OPEN",
      body = body,
    })
    model.next_issue = model.next_issue + 1
    if type(model.issue_create_yield) == "function" then
      model.issue_create_yield(model, issue_number)
    end
    return { stdout = "https://github.com/" .. tostring(repo) .. "/issues/" .. tostring(issue_number) .. "\n", stderr = "", exit_code = 0 }
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
  local old_now = now
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
    return options.now_seconds or 1780459324
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
  now = old_now
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

local function count_open_bridge_issues(github)
  local count = 0
  local issue_number = nil
  for _, issue in ipairs(github._model.issues or {}) do
    if tostring(issue.state or ""):upper() ~= "CLOSED" then
      count = count + 1
      issue_number = issue.number
    end
  end
  return count, issue_number
end

local function count_pr_bridge_markers(github, pr_number)
  local count = 0
  local marker_body = nil
  for _, comment in ipairs((github._model.prs[pr_number] or {}).comments or {}) do
    if tostring(comment.body or ""):find('external-pr-bridge:v1 repo="owner/repo" pr="' .. tostring(pr_number) .. '"', 1, true) ~= nil then
      count = count + 1
      marker_body = comment.body
    end
  end
  return count, marker_body
end

local function resume_thread(thread)
  local ok, value = coroutine.resume(thread)
  if not ok then
    error(value, 0)
  end
  return value
end

return {
  test_body_file_path_flattens_slash_bearing_repo = function()
    local core = require("core")
    local prefix = "/tmp/fkst-github-external-pr-intake-"
    local path = core.body_file_path("ChronoAIProject/fkst-packages", 1151, "issue")
    t.eq(path:sub(1, #prefix), prefix)
    t.is_true(path:sub(#prefix + 1):find("/", 1, true) == nil)
  end,

  test_scan_source_must_be_reliable = function()
    local spec = load_department().spec
    for _, queue in ipairs(spec.ephemeral or {}) do
      t.is_true(queue ~= "external_pr_scan")
    end
  end,

  test_candidate_activation_must_be_ephemeral = function()
    local spec = load_department().spec
    t.eq(#(spec.ephemeral or {}), 1)
    t.eq(spec.ephemeral[1], "external_pr_candidate")
  end,

  test_existing_intake_surfaces_cannot_schedule_external_pr_bridge = function()
    local proxy_raiser = read_disk_file(sibling_package_root("github-proxy") .. "/raisers/github_poll.lua")
    local proxy_poll = read_disk_file(sibling_package_root("github-proxy") .. "/departments/github_poll/main.lua")
    local proxy_issue_create = read_disk_file(sibling_package_root("github-proxy") .. "/departments/github_issue_create/main.lua")
    local proxy_issue_create_core = read_disk_file(sibling_package_root("github-proxy") .. "/core/issue_create.lua")
    local devloop_admission = read_disk_file(sibling_package_root("github-devloop-intake") .. "/departments/admission/main.lua")
    local external_scan_raiser = read_disk_file(package_root .. "/raisers/external_pr_scan.lua")
    local external_intake = read_disk_file(package_root .. "/departments/external_pr_intake/main.lua")

    -- Necessity proof: `github-proxy` can observe generic PR facts and execute
    -- already-formed issue-create effects, but it has no policy owner for the
    -- required middle step: external PR selection -> bridge issue materialization.
    t.is_true(proxy_raiser:find('produces = "github_poll_tick"', 1, true) ~= nil)
    t.is_true(proxy_raiser:find("external_pr_scan", 1, true) == nil)
    t.is_true(proxy_poll:find('consumes = { "github_poll_tick" }', 1, true) ~= nil)
    t.is_true(proxy_poll:find('produces = { "github_entity_changed" }', 1, true) ~= nil)
    t.is_true(proxy_poll:find('{ type = "pr"', 1, true) ~= nil)
    t.is_true(proxy_poll:find('raise("github_entity_changed"', 1, true) ~= nil)
    t.is_true(proxy_poll:find("core.is_external_candidate", 1, true) == nil)
    t.is_true(proxy_poll:find('"github_issue_create_request"', 1, true) == nil)
    t.is_true(proxy_poll:find("external_pr_candidate", 1, true) == nil)

    t.is_true(proxy_issue_create:find('consumes = { "github_issue_create_request" }', 1, true) ~= nil)
    t.is_true(proxy_issue_create:find('produces = { "github_issue_blocked_by_request" }', 1, true) ~= nil)
    t.is_true(proxy_issue_create:find("core.write_issue_create_request", 1, true) ~= nil)
    t.is_true(proxy_issue_create_core:find("payload.title", 1, true) ~= nil)
    t.is_true(proxy_issue_create_core:find("payload.body", 1, true) ~= nil)
    t.is_true(proxy_issue_create_core:find("payload.dedup_key", 1, true) ~= nil)
    t.is_true(proxy_issue_create_core:find("core.is_external_candidate", 1, true) == nil)
    t.is_true(proxy_issue_create_core:find("pr_list", 1, true) == nil)
    t.is_true(proxy_issue_create_core:find("external-pr-bridge:v1", 1, true) == nil)

    -- Devloop issue intake admits only GitHub issues already surfaced by the
    -- proxy entity stream; it has no PR source or bridge materialization policy.
    t.is_true(devloop_admission:find('"github-proxy.github_entity_changed"', 1, true) ~= nil)
    t.is_true(devloop_admission:find("devloop_intake_candidate", 1, true) ~= nil)
    t.is_true(devloop_admission:find("issue_list_intake", 1, true) == nil)
    t.is_true(devloop_admission:find("devloop_intake_tick", 1, true) == nil)
    t.is_true(devloop_admission:find("pr_list", 1, true) == nil)
    t.is_true(devloop_admission:find("#pr/", 1, true) == nil)

    -- The new adapter is the smallest owner of that middle step.
    t.is_true(external_scan_raiser:find('produces = "external_pr_scan"', 1, true) ~= nil)
    t.is_true(external_intake:find('consumes = { "external_pr_scan", "external_pr_candidate" }', 1, true) ~= nil)
    t.is_true(external_intake:find('produces = { "external_pr_candidate" }', 1, true) ~= nil)
    t.is_true(external_intake:find("github.pr_list(repo, 30)", 1, true) ~= nil)
    t.is_true(external_intake:find("core.is_external_candidate", 1, true) ~= nil)
    t.is_true(external_intake:find("with_lock(core.bridge_lock_key", 1, true) ~= nil)
    t.is_true(external_intake:find("external_pr_candidate", 1, true) ~= nil)
    t.is_true(external_intake:find("create_bridge_issue", 1, true) ~= nil)
    t.is_true(external_intake:find("write_comment", 1, true) ~= nil)
  end,

  test_bridge_lock_key_uses_production_cross_process_flock = function()
    local core = require("core")
    local runtime_root = os.getenv("FKST_RUNTIME_ROOT")
    t.is_true(runtime_root ~= nil and runtime_root ~= "")

    local lock_key = core.bridge_lock_key("owner/repo", 7)
    local lock_path = runtime_root .. "/locks/" .. lock_key .. "/=lock"
    local scratch = runtime_root .. "/external-pr-intake-lock-proof-" .. tostring(now())
    mkdir_p(parent_dir(lock_path))
    mkdir_p(scratch)

    local ready_path = scratch .. "/ready"
    local release_path = scratch .. "/release"
    local released_path = scratch .. "/released"
    local entered_path = scratch .. "/entered"
    local locker_script = scratch .. "/hold_lock.py"
    local releaser_script = scratch .. "/release_lock.py"

    write_disk_file(locker_script, [[
import fcntl
import os
import pathlib
import sys
import time

lock_path, ready_path, release_path, released_path = sys.argv[1:5]
pathlib.Path(os.path.dirname(lock_path)).mkdir(parents=True, exist_ok=True)
with open(lock_path, "a+", encoding="utf-8") as handle:
    fcntl.flock(handle.fileno(), fcntl.LOCK_EX)
    pathlib.Path(ready_path).write_text("ready\n", encoding="utf-8")
    deadline = time.time() + 10.0
    while not os.path.exists(release_path):
        if time.time() > deadline:
            pathlib.Path(released_path).write_text("timeout\n", encoding="utf-8")
            raise SystemExit(2)
        time.sleep(0.02)
    pathlib.Path(released_path).write_text("released\n", encoding="utf-8")
]])
    write_disk_file(releaser_script, [[
import pathlib
import sys
import time

release_path = sys.argv[1]
time.sleep(0.2)
pathlib.Path(release_path).write_text("release\n", encoding="utf-8")
]])

    local ok, err = pcall(function()
      start_python_background(locker_script, { lock_path, ready_path, release_path, released_path })
      if not wait_for_file(ready_path, 150) then
        error("github-external-pr-intake: lock helper did not acquire the bridge lock")
      end
      start_python_background(releaser_script, { release_path })

      local entered_after_release = false
      with_lock(lock_key, function()
        entered_after_release = file.exists(released_path)
        file.write(entered_path, tostring(entered_after_release))
      end)

      t.eq(read_disk_file(released_path), "released\n")
      t.eq(read_disk_file(entered_path), "true")
      t.is_true(entered_after_release)
    end)

    write_disk_file(release_path, "release\n")
    os.execute("sleep 0.05")
    if not ok then
      error(err, 0)
    end
  end,

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

  test_failed_ephemeral_candidate_is_rederived_by_next_scan = function()
    local github = new_fake_github({
      fail_pr_cli_view_once = true,
    })
    local ok, err = pcall(function()
      run_pipeline({
        github = github,
        event = candidate_event(7),
      })
    end)
    t.eq(ok, false)
    t.is_true(tostring(err or ""):find("transient PR view failure", 1, true) ~= nil)
    t.eq(count_kind(github._model.writes, "issue_create"), 0)
    t.eq(count_kind(github._model.writes, "pr_comment"), 0)

    local scan = run_pipeline({
      github = github,
      event = { queue = "external_pr_scan", payload = { schema = "github-external-pr-intake.v1" } },
    })
    t.eq(#scan.raises, 1)
    t.eq(scan.raises[1].queue, "external_pr_candidate")
    t.eq(scan.raises[1].payload.source_ref.kind, "external")
    t.eq(scan.raises[1].payload.source_ref.ref, "owner/repo#pr/7")

    run_pipeline({
      github = github,
      event = {
        queue = scan.raises[1].queue,
        payload = scan.raises[1].payload,
      },
    })
    t.eq(count_kind(github._model.writes, "issue_create"), 1)
    t.eq(count_kind(github._model.writes, "pr_comment"), 1)
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

  test_stale_visibility_reconciles_to_one_open_bridge_issue_and_one_marker = function()
    local github = new_fake_github({
      next_issue = 88,
      hidden_issues_until_creates = 2,
      hidden_comments_until_creates = 2,
    })
    run_pipeline({
      github = github,
      event = candidate_event(7),
    })
    run_pipeline({
      github = github,
      event = candidate_event(7),
    })

    local open_bridges, open_issue_number = count_open_bridge_issues(github)
    local trusted_markers, marker_body = count_pr_bridge_markers(github, 7)

    t.eq(count_kind(github._model.writes, "issue_create"), 2)
    t.eq(count_kind(github._model.writes, "issue_close"), 1)
    t.eq(open_bridges, 1)
    t.eq(open_issue_number, 88)
    t.eq(trusted_markers, 1)
    t.is_true(tostring(marker_body or ""):find('issue="88"', 1, true) ~= nil)
  end,

  test_same_bot_concurrent_candidates_serialize_to_one_bridge_issue_and_marker = function()
    local core = require("core")
    local github = new_fake_github({
      next_issue = 88,
      issue_create_yield = function()
        coroutine.yield("after-issue-create")
      end,
    })
    local files = {}
    local raises = {}
    local locks = {}
    local lock_busy = false
    local second_worker_waited = false
    local function bridge_lock(key, fn)
      table.insert(locks, key)
      while lock_busy do
        second_worker_waited = true
        coroutine.yield("waiting-for-lock")
      end
      lock_busy = true
      local result = fn()
      lock_busy = false
      return result
    end

    local old_file = file
    local old_log = log
    local old_raise = raise
    local old_with_lock = with_lock
    local old_pipeline = pipeline
    local old_now = now
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
    with_lock = bridge_lock
    now = function()
      return 1780459324
    end
    core.read_env = function(name)
      return ({
        FKST_GITHUB_REPO = "owner/repo",
        FKST_GITHUB_WRITE = "1",
        FKST_GITHUB_BOT_LOGIN = "fkst-test-bot",
        FKST_DEVLOOP_MANAGED_BOT_LOGINS = "fkst-test-bot,other-bot",
      })[name] or ""
    end

    local ok, err = pcall(function()
      local first_module = load_department()
      local second_module = load_department()
      local first_dept = first_module.make_department({ github = github })
      local second_dept = second_module.make_department({ github = github })
      local first = coroutine.create(function()
        first_dept.pipeline(candidate_event(7))
      end)
      local second = coroutine.create(function()
        second_dept.pipeline(candidate_event(7))
      end)

      t.eq(resume_thread(first), "after-issue-create")
      t.eq(resume_thread(second), "waiting-for-lock")
      t.eq(coroutine.status(first), "suspended")
      t.eq(coroutine.status(second), "suspended")
      resume_thread(first)
      t.eq(coroutine.status(first), "dead")
      resume_thread(second)
      t.eq(coroutine.status(second), "dead")
    end)
    core.read_env = old_read
    now = old_now
    pipeline = old_pipeline
    file = old_file
    log = old_log
    raise = old_raise
    with_lock = old_with_lock
    if not ok then
      error(err, 0)
    end

    local open_bridges, open_issue_number = count_open_bridge_issues(github)
    local trusted_markers, marker_body = count_pr_bridge_markers(github, 7)

    t.eq(count_kind(github._model.writes, "issue_create"), 1)
    t.eq(count_kind(github._model.writes, "issue_close"), 0)
    t.eq(count_kind(github._model.writes, "pr_comment"), 1)
    t.eq(open_bridges, 1)
    t.eq(open_issue_number, 88)
    t.eq(trusted_markers, 1)
    t.is_true(tostring(marker_body or ""):find('issue="88"', 1, true) ~= nil)
    t.eq(#locks, 2)
    t.eq(locks[1], core.bridge_lock_key("owner/repo", 7))
    t.eq(locks[2], core.bridge_lock_key("owner/repo", 7))
    t.eq(second_worker_waited, true)
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
