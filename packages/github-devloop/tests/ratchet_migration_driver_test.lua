local t = fkst.test

local function package_root()
  local source = package.searchpath("tests.ratchet_migration_driver_test", package.path)
  return source:match("(.+)/tests/ratchet_migration_driver_test%.lua$")
end

local function load_department()
  local old_pipeline = pipeline
  local module = dofile(package_root() .. "/departments/ratchet_migration_driver/main.lua")
  pipeline = old_pipeline
  return module
end

local function json_string(value)
  return require("std.strings").json_string(value)
end

local function plan_json(status, dedup_key)
  if status == "inventory_empty" then
    return '{"schema_version":"fkst.ratchet-slice.v1","ratchet":"saga-handler","allowlist_path":"migration/saga-handler.allowlist","remaining_count":0,"slice_size":3,"status":"inventory_empty","next_slice":null}\n'
  end
  local body = "# saga handler allowlist migration slice\n\nMachine-filed ratchet slice issue.\n\n<!-- fkst:github-proxy:issue-create:" .. dedup_key .. " -->\n<!-- fkst:ratchet-slice:v1 ratchet=\"saga-handler\" dedup=\"" .. dedup_key .. "\" -->\n"
  return '{"schema_version":"fkst.ratchet-slice.v1","ratchet":"saga-handler","allowlist_path":"migration/saga-handler.allowlist","remaining_count":3,"slice_size":3,"status":"slice_available","next_slice":{"dedup_key":'
    .. json_string(dedup_key)
    .. ',"sites":[],"title":"saga handler allowlist migration slice: abc123","body":'
    .. json_string(body)
    .. ',"labels":["fkst-dev:enabled"]}}\n'
end

local function parent_json(comments, state)
  local parts = {}
  for _, comment in ipairs(comments or {}) do
    table.insert(parts, '{"body":' .. json_string(comment.body) .. ',"author":{"login":' .. json_string(comment.author_login or "fkst-test-bot") .. '}}')
  end
  return '{"number":979,"state":' .. json_string(state or "OPEN") .. ',"comments":[' .. table.concat(parts, ",") .. ']}\n'
end

local function new_fake_github(opts)
  local options = opts or {}
  local model = {
    writes = {},
    searches = {},
    created_stdout = options.created_stdout or "https://github.com/owner/repo/issues/120\n",
  }
  local handle = { _model = model }
  function handle.issue_view(repo, issue_number, fields, timeout)
    table.insert(model.writes, { kind = "issue_view", repo = repo, issue_number = issue_number, fields = fields, timeout = timeout })
    return { stdout = parent_json(options.parent_comments, options.parent_state), stderr = "", exit_code = 0 }
  end
  function handle.issue_search(repo, query, fields, timeout)
    table.insert(model.searches, query)
    table.insert(model.writes, { kind = "issue_search", repo = repo, query = query, fields = fields, timeout = timeout })
    local stdout = "[]\n"
    if options.open_slice and query:find("fkst:ratchet-slice:v1", 1, true) ~= nil then
      stdout = '[{"number":121,"state":"OPEN","author":{"login":"fkst-test-bot"},"body":"<!-- fkst:ratchet-slice:v1 ratchet=\\"saga-handler\\" dedup=\\"old\\" -->"}]\n'
    elseif options.existing_slice and query:find("fkst:github-proxy:issue-create:", 1, true) ~= nil then
      stdout = '[{"number":122,"state":"CLOSED","author":{"login":"fkst-test-bot"},"body":' .. json_string(query) .. '}]\n'
    end
    return { stdout = stdout, stderr = "", exit_code = 0 }
  end
  function handle.issue_comment(repo, issue_number, body_file, timeout)
    table.insert(model.writes, { kind = "issue_comment", repo = repo, issue_number = issue_number, body_file = body_file, body = file.read(body_file), timeout = timeout })
    return { stdout = "", stderr = "", exit_code = 0 }
  end
  function handle.issue_create(repo, title, body_file, labels, assignees, timeout)
    table.insert(model.writes, { kind = "issue_create", repo = repo, title = title, body_file = body_file, body = file.read(body_file), labels = labels, assignees = assignees, timeout = timeout })
    return { stdout = model.created_stdout, stderr = "", exit_code = 0 }
  end
  function handle.issue_close(repo, issue_number, timeout)
    table.insert(model.writes, { kind = "issue_close", repo = repo, issue_number = issue_number, timeout = timeout })
    return { stdout = "", stderr = "", exit_code = 0 }
  end
  return handle
end

local function run_driver(opts)
  local options = opts or {}
  local github = new_fake_github(options.github)
  local exec_calls = {}
  local old_exec_argv = exec_argv
  local old_file = file
  local files = {}
  local old_log = log
  exec_argv = function(argv, timeout)
    table.insert(exec_calls, { argv = argv, timeout = timeout })
    return { stdout = options.plan or plan_json("slice_available", "saga-handler/slice/abc123"), stderr = "", exit_code = 0 }
  end
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
  local module = load_department()
  local dept = module.make_department({ github = github })
  local env = options.env or {
    FKST_GITHUB_REPO = "owner/repo",
    FKST_GITHUB_WRITE = "1",
    FKST_GITHUB_BOT_LOGIN = "fkst-test-bot",
  }
  local old_core = package.loaded.core
  local core = require("core")
  local old_read = core.read_env
  core.read_env = function(name)
    return env[name] or ""
  end
  local ok, err = pcall(function()
    dept.pipeline({
      queue = "ratchet_migration_poll",
      payload = { ratchet = "saga-handler" },
    })
  end)
  core.read_env = old_read
  package.loaded.core = old_core
  exec_argv = old_exec_argv
  file = old_file
  log = old_log
  if not ok then
    error(err, 0)
  end
  return { github = github, exec_calls = exec_calls, files = files, dept = dept }
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

return {
  test_poll_slice_available_files_one_issue_and_ledger = function()
    local result = run_driver()
    local writes = result.github._model.writes
    local created = write_of_kind(writes, "issue_create")
    local intent = write_of_kind(writes, "issue_comment", 1)
    local ledger = write_of_kind(writes, "issue_comment", 2)

    t.eq(count_kind(writes, "issue_create"), 1)
    t.eq(count_kind(writes, "issue_comment"), 2)
    t.is_true(created.body:find("Machine-filed ratchet slice issue.", 1, true) ~= nil)
    t.eq(created.labels[1], "fkst-dev:enabled")
    t.is_true(intent.body:find("issue-create-intent:v1", 1, true) ~= nil)
    t.is_true(ledger.body:find("issue-created:v1", 1, true) ~= nil)
    t.eq(result.exec_calls[1].argv[1], "python3")
    t.eq(result.exec_calls[1].argv[2], "scripts/ratchet_migration_slicer.py")
    t.eq(result.exec_calls[1].argv[3], "saga-handler")
    t.eq(result.exec_calls[1].argv[4], "--json")
    t.eq(result.exec_calls[1].argv[5], nil)
  end,

  test_poll_with_in_flight_slice_noops = function()
    local result = run_driver({
      github = { open_slice = true },
    })

    t.eq(count_kind(result.github._model.writes, "issue_create"), 0)
    t.eq(count_kind(result.github._model.writes, "issue_comment"), 0)
  end,

  test_poll_with_empty_inventory_closes_parent = function()
    local result = run_driver({
      plan = plan_json("inventory_empty"),
    })

    t.eq(count_kind(result.github._model.writes, "issue_close"), 1)
    t.eq(count_kind(result.github._model.writes, "issue_create"), 0)
  end,

  test_dry_run_does_not_write_github_mutations = function()
    local result = run_driver({
      env = {
        FKST_GITHUB_REPO = "owner/repo",
        FKST_GITHUB_WRITE = "",
        FKST_GITHUB_BOT_LOGIN = "fkst-test-bot",
      },
    })

    t.eq(count_kind(result.github._model.writes, "issue_create"), 0)
    t.eq(count_kind(result.github._model.writes, "issue_comment"), 0)
  end,
}
