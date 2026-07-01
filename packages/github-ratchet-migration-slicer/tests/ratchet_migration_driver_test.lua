local strings = require("contract.strings")
local t = fkst.test

local function load_department()
  local old_pipeline = pipeline
  local module = require("departments.ratchet_migration_driver.main")
  pipeline = old_pipeline
  return module
end

local function json_string(value)
  return strings.json_string(value)
end

local ratchet_allowlist_paths = {
  ["saga-handler"] = "migration/saga-handler.allowlist",
  ["code-dedup"] = "migration/code-dedup.allowlist",
}

local default_entry_key = "1111111111111111111111111111111111111111111111111111111111111111"
local stale_claim_time = "2000-01-01T00:00:00Z"

local function ratchet_title(ratchet)
  return tostring(ratchet or "saga-handler"):gsub("%-", " ") .. " allowlist migration slice"
end

local function plan_json(status, dedup_key, ratchet, entry_key)
  ratchet = ratchet or "saga-handler"
  entry_key = entry_key or default_entry_key
  local allowlist_path = ratchet_allowlist_paths[ratchet]
  if allowlist_path == nil then
    error("unknown test ratchet: " .. tostring(ratchet))
  end
  if status == "inventory_empty" then
    return '{"schema_version":"fkst.ratchet-slice.v1","ratchet":'
      .. json_string(ratchet)
      .. ',"allowlist_path":'
      .. json_string(allowlist_path)
      .. ',"remaining_count":0,"slice_size":3,"status":"inventory_empty","next_slice":null}\n'
  end
  local title = ratchet_title(ratchet)
  local body = "# "
    .. title
    .. "\n\nMachine-filed ratchet slice issue.\n\n<!-- fkst:github-proxy:issue-create:"
    .. dedup_key
    .. " -->\n<!-- fkst:ratchet-slice:v1 ratchet=\""
    .. ratchet
    .. "\" dedup=\""
    .. dedup_key
    .. "\" entries=\""
    .. entry_key
    .. "\" entry_key=\""
    .. entry_key
    .. "\" allowlist_path=\""
    .. allowlist_path
    .. "\" generation=\"1\" coord_ref=\"refs/fkst/migration-slices/"
    .. entry_key
    .. "\" -->\n"
  return '{"schema_version":"fkst.ratchet-slice.v1","ratchet":'
    .. json_string(ratchet)
    .. ',"allowlist_path":'
    .. json_string(allowlist_path)
    .. ',"remaining_count":3,"slice_size":3,"status":"slice_available","next_slice":{"dedup_key":'
    .. json_string(dedup_key)
    .. ',"sites":[{"entry_key":'
    .. json_string(entry_key)
    .. '}],"title":'
    .. json_string(title .. ": abc123")
    .. ',"body":'
    .. json_string(body)
    .. ',"labels":["fkst-dev:enabled"]}}\n'
end

local function parent_json(comments, state)
  local parts = {}
  for _, comment in ipairs(comments or {}) do
    local created = comment.created_at and (',"createdAt":' .. json_string(comment.created_at)) or ""
    table.insert(parts, '{"body":' .. json_string(comment.body) .. ',"author":{"login":' .. json_string(comment.author_login or "fkst-test-bot") .. '}' .. created .. "}")
  end
  return '{"number":979,"state":' .. json_string(state or "OPEN") .. ',"comments":[' .. table.concat(parts, ",") .. ']}\n'
end

local function issue_json(number, state, body_text, author_login)
  return '{"number":'
    .. tostring(number or 121)
    .. ',"state":'
    .. json_string(state or "OPEN")
    .. ',"author":{"login":'
    .. json_string(author_login or "fkst-test-bot")
    .. '},"body":'
    .. json_string(body_text or "")
    .. "}\n"
end

local function query_ratchet(query)
  return tostring(query or ""):match('ratchet="([^"]+)"')
end

local function new_fake_github(opts)
  local options = opts or {}
  local dedup_key = options.dedup_key or "saga-handler/slice/abc123"
  local model = {
    writes = {},
    searches = {},
    created_stdout = options.created_stdout or "https://github.com/owner/repo/issues/120\n",
  }
  local handle = { _model = model }
  function handle.issue_view(repo, issue_number, fields, timeout)
    table.insert(model.writes, { kind = "issue_view", repo = repo, issue_number = issue_number, fields = fields, timeout = timeout })
    if options.child_issues and options.child_issues[issue_number] then
      local child = options.child_issues[issue_number]
      return {
        stdout = issue_json(issue_number, child.state, child.body, child.author_login),
        stderr = "",
        exit_code = 0,
      }
    end
    return { stdout = parent_json(options.parent_comments, options.parent_state), stderr = "", exit_code = 0 }
  end
  function handle.issue_search(repo, query, fields, timeout)
    table.insert(model.searches, query)
    table.insert(model.writes, { kind = "issue_search", repo = repo, query = query, fields = fields, timeout = timeout })
    local stdout = "[]\n"
    if options.open_slice and query:find("fkst:ratchet-slice:v1", 1, true) ~= nil then
      local ratchet = options.open_slice_all_ratchets and query_ratchet(query) or options.open_slice_ratchet or "saga-handler"
      stdout = '[{"number":121,"state":"OPEN","author":{"login":'
        .. json_string(options.open_slice_author_login or "fkst-test-bot")
        .. '},"body":"<!-- fkst:ratchet-slice:v1 ratchet=\\"'
        .. ratchet
        .. '\\" dedup=\\"'
        .. dedup_key
        .. '\\" entries=\\"'
        .. (options.open_slice_entry_key or default_entry_key)
        .. '\\" entry_key=\\"'
        .. (options.open_slice_entry_key or default_entry_key)
        .. '\\" -->"}]\n'
    elseif options.existing_slice and query:find("fkst:github-proxy:issue-create:", 1, true) ~= nil then
      stdout = '[{"number":122,"state":'
        .. json_string(options.existing_slice_state or "OPEN")
        .. ',"author":{"login":'
        .. json_string(options.existing_slice_author_login or "fkst-test-bot")
        .. '},"body":'
        .. json_string(query)
        .. '}]\n'
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
  function handle.issue_add_sub_issue(repo, parent_issue_number, sub_issue_number, timeout)
    table.insert(model.writes, { kind = "issue_add_sub_issue", repo = repo, parent_issue_number = parent_issue_number, sub_issue_number = sub_issue_number, timeout = timeout })
    return { stdout = "", stderr = "", exit_code = 0 }
  end
  function handle.issue_close(repo, issue_number, timeout)
    table.insert(model.writes, { kind = "issue_close", repo = repo, issue_number = issue_number, timeout = timeout })
    return { stdout = "", stderr = "", exit_code = 0 }
  end
  return handle
end

local function commit_stdout(data)
  return "tree 0000000000000000000000000000000000000000\n\n" .. tostring(data or "{}") .. "\n"
end

local function ledger_json(fields)
  local values = fields or {}
  local state = values.state or "issue-created"
  local generation = values.generation or 1
  local issue_number = values.issue_number
  local issue = issue_number == nil and "null" or tostring(issue_number)
  return "{"
    .. '"schema":"fkst.ratchet-migration-slice-ledger.v1"'
    .. ',"state":' .. json_string(state)
    .. ',"entry_key":' .. json_string(values.entry_key or default_entry_key)
    .. ',"allowlist_path":"migration/saga-handler.allowlist"'
    .. ',"generation":' .. tostring(generation)
    .. ',"claim_owner":' .. json_string(values.claim_owner or "fkst-test-bot")
    .. ',"claimed_at":' .. json_string(values.claimed_at or "2026-06-19T00:00:00Z")
    .. ',"issue_number":' .. issue
    .. ',"updated_at":' .. json_string(values.updated_at or "2026-06-19T00:00:00Z")
    .. "}"
end

local function new_fake_git(opts)
  local options = opts or {}
  local model = {
    writes = {},
    commits = {},
    fetched_refs = {},
    fetched_shas = {},
    ref_sha = options.ref_sha,
    push_fail = options.push_fail,
  }
  local handle = { _model = model }
  function handle.ls_remote_ref(remote, ref, timeout)
    table.insert(model.writes, { kind = "ls_remote_ref", remote = remote, ref = ref, timeout = timeout })
    if model.ref_sha == nil then
      return { stdout = "", stderr = "", exit_code = 0 }
    end
    return { stdout = model.ref_sha .. "\t" .. ref .. "\n", stderr = "", exit_code = 0 }
  end
  function handle.fetch_ref(remote, ref, timeout)
    table.insert(model.writes, { kind = "fetch_ref", remote = remote, ref = ref, timeout = timeout })
    model.fetched_refs[ref] = true
    if model.ref_sha ~= nil then
      model.fetched_shas[model.ref_sha] = true
    end
    return { stdout = "", stderr = "", exit_code = 0 }
  end
  function handle.cat_file_pretty(ref, timeout)
    table.insert(model.writes, { kind = "cat_file_pretty", ref = ref, timeout = timeout })
    if model.ref_sha == ref and not model.fetched_shas[ref] then
      return { stdout = "", stderr = "missing object", exit_code = 1 }
    end
    return { stdout = commit_stdout(options.ledger_json or ledger_json({ issue_number = 121 })), stderr = "", exit_code = 0 }
  end
  function handle.rev_parse_ref_tree(ref, timeout)
    table.insert(model.writes, { kind = "rev_parse_ref_tree", ref = ref, timeout = timeout })
    return { stdout = "2222222222222222222222222222222222222222\n", stderr = "", exit_code = 0 }
  end
  function handle.commit_tree(tree_sha, parent_sha, message_file, timeout)
    table.insert(model.writes, { kind = "commit_tree", tree_sha = tree_sha, parent_sha = parent_sha, message_file = message_file, body = file.read(message_file), timeout = timeout })
    local sha = string.format("%040d", #model.commits + 1)
    table.insert(model.commits, { sha = sha, parent_sha = parent_sha, body = file.read(message_file) })
    return { stdout = sha .. "\n", stderr = "", exit_code = 0 }
  end
  function handle.push_ref_update(remote, sha, ref, force_with_lease, timeout)
    table.insert(model.writes, { kind = "push_ref_update", remote = remote, sha = sha, ref = ref, force_with_lease = force_with_lease, timeout = timeout })
    if model.push_fail then
      return { stdout = "", stderr = "rejected", exit_code = 1 }
    end
    model.ref_sha = sha
    return { stdout = "", stderr = "", exit_code = 0 }
  end
  return handle
end

local function run_driver(opts)
  local options = opts or {}
  local github = new_fake_github(options.github)
  local git = new_fake_git(options.git)
  local exec_calls = {}
  local existing_files = options.existing_files or {
    ["migration/saga-handler.allowlist"] = true,
    ["migration/code-dedup.allowlist"] = true,
  }
  local old_exec_argv = exec_argv
  local old_file = file
  local files = {}
  local old_log = log
  exec_argv = function(spec)
    t.eq(type(spec), "table")
    t.eq(type(spec.argv), "table")
    t.eq(type(spec.timeout), "number")
    t.is_nil(spec.cmd)
    for _, value in ipairs(spec.argv) do
      t.eq(type(value), "string")
    end
    table.insert(exec_calls, spec)
    local requested_ratchet = spec.argv[3]
    local stdout = options.plan
    if stdout == nil then
      stdout = plan_json("slice_available", options.dedup_key or "saga-handler/slice/abc123", requested_ratchet)
    end
    return { stdout = stdout, stderr = "", exit_code = 0 }
  end
  file = {
    write = function(path, body)
      files[path] = body
    end,
    read = function(path)
      return files[path] or ""
    end,
    exists = function(path)
      return files[path] ~= nil or existing_files[path] == true
    end,
  }
  log = {
    info = function(_message) end,
    warn = function(_message) end,
    error = function(_message) end,
  }
  local module = load_department()
  local dept = module.make_department({ github = github, git = git })
  local env = options.env or {
    FKST_GITHUB_REPO = "owner/repo",
    FKST_GITHUB_WRITE = "1",
    FKST_GITHUB_BOT_LOGIN = "fkst-test-bot",
  }
  local core = require("core")
  local old_read = core.read_env
  core.read_env = function(name)
    return env[name] or ""
  end
  local ok, err = pcall(function()
    local payload = {}
    if not options.all_ratchets then
      payload.ratchet = options.ratchet or "saga-handler"
    end
    dept.pipeline({
      queue = "ratchet_migration_poll",
      payload = payload,
    })
  end)
  core.read_env = old_read
  exec_argv = old_exec_argv
  file = old_file
  log = old_log
  if not ok then
    error(err, 0)
  end
  return { github = github, git = git, exec_calls = exec_calls, files = files, dept = dept }
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

local function index_of_kind(writes, kind)
  for index, write in ipairs(writes or {}) do
    if write.kind == kind then
      return index
    end
  end
  return nil
end

return {
  test_poll_slice_available_files_one_issue_and_ledger = function()
    local result = run_driver()
    local writes = result.github._model.writes
    local created = write_of_kind(writes, "issue_create")
    local linked = write_of_kind(writes, "issue_add_sub_issue")
    local intent = write_of_kind(writes, "issue_comment", 1)
    local ledger = write_of_kind(writes, "issue_comment", 2)

    t.eq(count_kind(writes, "issue_create"), 1)
    t.eq(count_kind(writes, "issue_add_sub_issue"), 1)
    t.eq(count_kind(writes, "issue_comment"), 2)
    t.is_true(created.body:find("Machine-filed ratchet slice issue.", 1, true) ~= nil)
    t.eq(created.labels[1], "fkst-dev:enabled")
    t.eq(linked.parent_issue_number, 979)
    t.eq(linked.sub_issue_number, 120)
    t.is_true(intent.body:find("issue-create-intent:v1", 1, true) ~= nil)
    t.is_true(ledger.body:find("issue-created:v1", 1, true) ~= nil)
    t.eq(result.exec_calls[1].argv[1], "python3")
    t.is_true(result.exec_calls[1].argv[2]:find("packages/github-ratchet-migration-slicer/tools/ratchet_migration_slicer.py", 1, true) ~= nil)
    t.eq(result.exec_calls[1].argv[3], "saga-handler")
    t.eq(result.exec_calls[1].argv[4], "--repo-root")
    t.eq(result.exec_calls[1].argv[5], ".")
    t.eq(result.exec_calls[1].argv[6], "--json")
    t.eq(result.exec_calls[1].argv[7], nil)
    t.eq(result.exec_calls[1].timeout, 120)
  end,

  test_missing_allowlist_substrate_noops_without_planning = function()
    local result = run_driver({
      existing_files = {},
    })

    t.eq(#result.exec_calls, 0)
    t.eq(count_kind(result.github._model.writes, "issue_view"), 0)
    t.eq(count_kind(result.github._model.writes, "issue_create"), 0)
    t.eq(count_kind(result.github._model.writes, "issue_add_sub_issue"), 0)
    t.eq(count_kind(result.github._model.writes, "issue_comment"), 0)
    t.eq(count_kind(result.github._model.writes, "issue_close"), 0)
  end,

  test_poll_with_in_flight_slice_noops = function()
    local result = run_driver({
      github = { open_slice = true },
    })

    t.eq(count_kind(result.github._model.writes, "issue_create"), 0)
    t.eq(count_kind(result.github._model.writes, "issue_add_sub_issue"), 0)
    t.eq(count_kind(result.github._model.writes, "issue_comment"), 0)
  end,

  test_poll_with_managed_sibling_open_slice_noops = function()
    local result = run_driver({
      all_ratchets = true,
      env = {
        FKST_GITHUB_REPO = "owner/repo",
        FKST_GITHUB_WRITE = "1",
        FKST_GITHUB_BOT_LOGIN = "ElonSG",
        FKST_DEVLOOP_MANAGED_BOT_LOGINS = "ElonSG,loning",
      },
      github = {
        open_slice = true,
        open_slice_all_ratchets = true,
        open_slice_author_login = "loning[bot]",
        dedup_key = "saga-handler/slice/abc123",
      },
    })

    t.eq(count_kind(result.github._model.writes, "issue_create"), 0)
    t.eq(count_kind(result.github._model.writes, "issue_add_sub_issue"), 0)
    t.eq(count_kind(result.github._model.writes, "issue_comment"), 0)
    t.eq(count_kind(result.github._model.writes, "issue_search"), 2)
    t.eq(#result.exec_calls, 2)
  end,

  test_poll_with_ref_issue_created_open_child_noops = function()
    local result = run_driver({
      git = {
        ref_sha = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        ledger_json = ledger_json({ state = "issue-created", issue_number = 121 }),
      },
      github = {
        child_issues = {
          [121] = {
            author_login = "loning[bot]",
            state = "OPEN",
            body = '<!-- fkst:ratchet-slice:v1 ratchet="saga-handler" entry_key="' .. default_entry_key .. '" -->',
          },
        },
      },
    })

    t.eq(count_kind(result.github._model.writes, "issue_create"), 0)
    t.eq(count_kind(result.github._model.writes, "issue_add_sub_issue"), 0)
    t.eq(count_kind(result.github._model.writes, "issue_comment"), 0)
    t.eq(count_kind(result.git._model.writes, "push_ref_update"), 0)
    t.is_true(index_of_kind(result.git._model.writes, "fetch_ref") < index_of_kind(result.git._model.writes, "cat_file_pretty"))
  end,

  test_poll_with_fresh_ref_claim_noops = function()
    local result = run_driver({
      git = {
        ref_sha = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        ledger_json = ledger_json({ state = "claiming", claimed_at = os.date("!%Y-%m-%dT%H:%M:%SZ", os.time()) }),
      },
    })

    t.eq(count_kind(result.github._model.writes, "issue_create"), 0)
    t.eq(count_kind(result.github._model.writes, "issue_add_sub_issue"), 0)
    t.eq(count_kind(result.github._model.writes, "issue_comment"), 0)
    t.eq(count_kind(result.git._model.writes, "push_ref_update"), 0)
  end,

  test_poll_with_stale_ref_claim_recreates_generation = function()
    local result = run_driver({
      git = {
        ref_sha = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        ledger_json = ledger_json({ state = "claiming", claimed_at = stale_claim_time, generation = 2 }),
      },
    })

    t.eq(count_kind(result.github._model.writes, "issue_create"), 1)
    t.eq(count_kind(result.github._model.writes, "issue_add_sub_issue"), 1)
    t.eq(count_kind(result.github._model.writes, "issue_comment"), 2)
    t.eq(count_kind(result.git._model.writes, "push_ref_update"), 2)
    t.eq(write_of_kind(result.git._model.writes, "push_ref_update", 1).force_with_lease, false)
  end,

  test_poll_with_closed_ref_issue_recreates_generation = function()
    local result = run_driver({
      git = {
        ref_sha = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        ledger_json = ledger_json({ state = "issue-created", issue_number = 121, generation = 1 }),
      },
      github = {
        child_issues = {
          [121] = {
            author_login = "loning[bot]",
            state = "CLOSED",
            body = '<!-- fkst:ratchet-slice:v1 ratchet="saga-handler" entry_key="' .. default_entry_key .. '" -->',
          },
        },
      },
    })

    t.eq(count_kind(result.github._model.writes, "issue_create"), 1)
    t.eq(count_kind(result.github._model.writes, "issue_add_sub_issue"), 1)
    t.eq(count_kind(result.github._model.writes, "issue_comment"), 2)
    t.eq(count_kind(result.git._model.writes, "push_ref_update"), 2)
  end,

  test_poll_ref_push_race_noops_without_issue_create = function()
    local result = run_driver({
      git = {
        push_fail = true,
      },
    })

    t.eq(count_kind(result.github._model.writes, "issue_create"), 0)
    t.eq(count_kind(result.github._model.writes, "issue_add_sub_issue"), 0)
    t.eq(count_kind(result.github._model.writes, "issue_comment"), 0)
    t.eq(count_kind(result.git._model.writes, "push_ref_update"), 1)
  end,

  test_poll_with_open_slice_for_different_entry_creates = function()
    local result = run_driver({
      github = {
        open_slice = true,
        open_slice_entry_key = "2222222222222222222222222222222222222222222222222222222222222222",
      },
    })

    t.eq(count_kind(result.github._model.writes, "issue_create"), 1)
    t.eq(count_kind(result.github._model.writes, "issue_add_sub_issue"), 1)
    t.eq(count_kind(result.github._model.writes, "issue_comment"), 2)
  end,

  test_single_instance_login_treats_sibling_slice_as_shared_existence = function()
    local result = run_driver({
      env = {
        FKST_GITHUB_REPO = "owner/repo",
        FKST_GITHUB_WRITE = "1",
        FKST_GITHUB_BOT_LOGIN = "ElonSG",
      },
      github = {
        open_slice = true,
        open_slice_author_login = "loning[bot]",
        dedup_key = "saga-handler/slice/abc123",
      },
    })

    t.eq(count_kind(result.github._model.writes, "issue_create"), 0)
    t.eq(count_kind(result.github._model.writes, "issue_add_sub_issue"), 0)
    t.eq(count_kind(result.github._model.writes, "issue_comment"), 0)
  end,

  test_untrusted_matching_slice_marker_suppresses_creation_author_agnostic = function()
    local result = run_driver({
      all_ratchets = true,
      env = {
        FKST_GITHUB_REPO = "owner/repo",
        FKST_GITHUB_WRITE = "1",
        FKST_GITHUB_BOT_LOGIN = "ElonSG",
        FKST_DEVLOOP_MANAGED_BOT_LOGINS = "ElonSG,loning",
      },
      github = {
        open_slice = true,
        open_slice_all_ratchets = true,
        open_slice_author_login = "randomuser",
        dedup_key = "saga-handler/slice/abc123",
      },
    })

    t.eq(count_kind(result.github._model.writes, "issue_create"), 0)
    t.eq(count_kind(result.github._model.writes, "issue_add_sub_issue"), 0)
    t.eq(count_kind(result.github._model.writes, "issue_comment"), 0)
  end,

  test_poll_with_empty_inventory_closes_parent = function()
    local result = run_driver({
      plan = plan_json("inventory_empty"),
    })

    t.eq(count_kind(result.github._model.writes, "issue_close"), 1)
    t.eq(count_kind(result.github._model.writes, "issue_create"), 0)
    t.eq(count_kind(result.github._model.writes, "issue_add_sub_issue"), 0)
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
    t.eq(count_kind(result.github._model.writes, "issue_add_sub_issue"), 0)
    t.eq(count_kind(result.github._model.writes, "issue_comment"), 0)
  end,

  test_empty_current_login_keeps_permissive_author_dedup = function()
    local result = run_driver({
      all_ratchets = true,
      env = {
        FKST_GITHUB_REPO = "owner/repo",
        FKST_GITHUB_WRITE = "",
        FKST_GITHUB_BOT_LOGIN = "",
        FKST_DEVLOOP_MANAGED_BOT_LOGINS = "ElonSG,loning",
      },
      github = {
        open_slice = true,
        open_slice_all_ratchets = true,
        open_slice_author_login = "randomuser",
        dedup_key = "saga-handler/slice/abc123",
      },
    })

    t.eq(count_kind(result.github._model.writes, "issue_create"), 0)
    t.eq(count_kind(result.github._model.writes, "issue_add_sub_issue"), 0)
    t.eq(count_kind(result.github._model.writes, "issue_comment"), 0)
    t.eq(count_kind(result.github._model.writes, "issue_search"), 2)
  end,
}
