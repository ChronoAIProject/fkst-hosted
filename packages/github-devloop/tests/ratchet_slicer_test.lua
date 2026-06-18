local t = fkst.test
local core = require("core")

local function find_raise(raises, queue)
  for _, raised in ipairs(raises or {}) do
    if raised.queue == queue then
      return raised
    end
  end
  return nil
end

local function count_raises(raises, queue)
  local count = 0
  for _, raised in ipairs(raises or {}) do
    if raised.queue == queue then
      count = count + 1
    end
  end
  return count
end

local function site_json(path, line, detail)
  return '{"detail":"' .. detail .. '","line":' .. tostring(line)
    .. ',"path":"' .. path .. '","site_ref":"' .. path .. ':' .. tostring(line) .. '"}'
end

local function slice_json(ratchet, opts)
  local options = opts or {}
  local selected = options.selected_count or 1
  local current = options.current_count or selected
  local fingerprint = options.fingerprint or (ratchet .. "-abc123")
  local parent = options.parent_issue or (ratchet == "code-dedup" and 1002 or 892)
  local sites = {}
  if selected > 0 then
    for index = 1, selected do
      table.insert(sites, site_json("packages/example/" .. ratchet .. tostring(index) .. ".lua", index, "free_form_pipeline"))
    end
  end
  return '{"allowlist_path":"migration/' .. ratchet .. '.allowlist"'
    .. ',"current_count":' .. tostring(current)
    .. ',"dedup_key":"' .. ratchet .. '/slice/' .. fingerprint .. '"'
    .. ',"migration_kind":"allowlist"'
    .. ',"parent_issue":' .. tostring(parent)
    .. ',"ratchet":"' .. ratchet .. '"'
    .. ',"reference_shape":"Use the already migrated reference shape."'
    .. ',"schema":"fkst.ratchet-slice.v1"'
    .. ',"selected_count":' .. tostring(selected)
    .. ',"sites":[' .. table.concat(sites, ",") .. ']'
    .. ',"sites_fingerprint":"' .. fingerprint .. '"'
    .. ',"slice_size":3'
    .. ',"target_count":0'
    .. ',"title":"' .. ratchet .. ' allowlist migration slice"}\n'
end

local function mock_env(repo, write)
  t.mock_command('printf %s "$FKST_GITHUB_REPO"', {
    stdout = repo or "owner/repo",
    stderr = "",
    exit_code = 0,
  })
  t.mock_command('printf %s "$FKST_GITHUB_WRITE"', {
    stdout = write or "",
    stderr = "",
    exit_code = 0,
  })
end

local function ratchet_search_cmd(ratchet)
  return "gh issue list --repo owner/repo --state all --limit 100 --search 'fkst:github-devloop:ratchet-slice:v1 ratchet=\""
    .. tostring(ratchet)
    .. "\"' --json 'number,title,state,body'"
end

local function mock_slicer_docs(saga_doc, dedup_doc)
  t.mock_command("pwd", {
    stdout = "/repo\n",
    stderr = "",
    exit_code = 0,
  })
  t.mock_command("python3 -B scripts/ratchet_migration_slicer.py saga-handler --repo-root /repo --slice-size 3 --json", {
    stdout = saga_doc,
    stderr = "",
    exit_code = 0,
  })
  t.mock_command(ratchet_search_cmd("saga-handler"), {
    stdout = "[]\n",
    stderr = "",
    exit_code = 0,
  })
  t.mock_command("pwd", {
    stdout = "/repo\n",
    stderr = "",
    exit_code = 0,
  })
  t.mock_command("python3 -B scripts/ratchet_migration_slicer.py code-dedup --repo-root /repo --slice-size 3 --json", {
    stdout = dedup_doc,
    stderr = "",
    exit_code = 0,
  })
  t.mock_command(ratchet_search_cmd("code-dedup"), {
    stdout = "[]\n",
    stderr = "",
    exit_code = 0,
  })
end

local function run_scan(extra_env)
  return t.run_department("departments/ratchet_slice_scan/main.lua", {
    queue = "devloop_branch_tick",
    payload = { schema = "github-devloop.branch-tick.v1" },
  }, {
    env = extra_env or {
      FKST_GITHUB_REPO = "owner/repo",
      FKST_RUNTIME_ROOT = "/tmp/fkst-packages-test/ratchet-slicer",
    },
  })
end

return {
  test_build_request_contains_marker_key_and_parent_block = function()
    local doc = json.decode(slice_json("saga-handler", { fingerprint = "fp1", parent_issue = 979 }))

    local request = core.build_ratchet_slice_issue_create_request("owner/repo", doc)

    t.eq(request.schema, "github-proxy.issue-create.v1")
    t.eq(request.dedup_key, "saga-handler/slice/fp1")
    t.eq(request.parent_comment_target.issue_number, 979)
    t.eq(request.post_create_blocked_by.blocked_issue_number, 979)
    t.eq(request.post_create_blocked_by.dedup_key, "saga-handler/slice/fp1/blocked-by")
    t.is_true(request.body:find('fkst:github-devloop:ratchet-slice:v1 ratchet="saga-handler" sites_fingerprint="fp1"', 1, true) ~= nil)
    t.is_true(request.body:find("`scripts/run.sh test` exits 0", 1, true) ~= nil)
  end,

  test_scan_raises_one_issue_create_per_nonempty_registered_ratchet = function()
    mock_env("owner/repo")
    mock_slicer_docs(
      slice_json("saga-handler", { fingerprint = "saga-fp", parent_issue = 979 }),
      slice_json("code-dedup", { fingerprint = "dedup-fp", parent_issue = 1002 })
    )

    local result = run_scan()

    t.eq(result.exit_code, 0)
    t.eq(count_raises(result.raises, "github-proxy.github_issue_create_request"), 2)
    local first = result.raises[1].payload
    t.eq(first.dedup_key, "saga-handler/slice/saga-fp")
    t.eq(first.parent_comment_target.issue_number, 979)
    local second = result.raises[2].payload
    t.eq(second.dedup_key, "code-dedup/slice/dedup-fp")
    t.eq(second.parent_comment_target.issue_number, 1002)
  end,

  test_scan_suppresses_ratchet_when_open_slice_exists = function()
    mock_env("owner/repo")
    t.mock_command("pwd", { stdout = "/repo\n", stderr = "", exit_code = 0 })
    t.mock_command("python3 -B scripts/ratchet_migration_slicer.py saga-handler --repo-root /repo --slice-size 3 --json", {
      stdout = slice_json("saga-handler", { fingerprint = "saga-fp", parent_issue = 979 }),
      stderr = "",
      exit_code = 0,
    })
    t.mock_command(ratchet_search_cmd("saga-handler"), {
      stdout = '[{"number":55,"state":"OPEN","title":"slice","body":"<!-- fkst:github-devloop:ratchet-slice:v1 ratchet=\\"saga-handler\\" sites_fingerprint=\\"old\\" -->"}]\n',
      stderr = "",
      exit_code = 0,
    })
    t.mock_command("pwd", { stdout = "/repo\n", stderr = "", exit_code = 0 })
    t.mock_command("python3 -B scripts/ratchet_migration_slicer.py code-dedup --repo-root /repo --slice-size 3 --json", {
      stdout = slice_json("code-dedup", { fingerprint = "dedup-fp", parent_issue = 1002 }),
      stderr = "",
      exit_code = 0,
    })
    t.mock_command(ratchet_search_cmd("code-dedup"), {
      stdout = "[]\n",
      stderr = "",
      exit_code = 0,
    })

    local result = run_scan()

    t.eq(result.exit_code, 0)
    t.eq(count_raises(result.raises, "github-proxy.github_issue_create_request"), 1)
    t.eq(find_raise(result.raises, "github-proxy.github_issue_create_request").payload.dedup_key, "code-dedup/slice/dedup-fp")
  end,

  test_empty_ratchet_closes_parent_only_in_real_write_mode = function()
    mock_env("owner/repo", "1")
    t.mock_command("pwd", { stdout = "/repo\n", stderr = "", exit_code = 0 })
    t.mock_command("python3 -B scripts/ratchet_migration_slicer.py saga-handler --repo-root /repo --slice-size 3 --json", {
      stdout = slice_json("saga-handler", { selected_count = 0, current_count = 0, fingerprint = "empty", parent_issue = 979 }),
      stderr = "",
      exit_code = 0,
    })
    t.mock_command(ratchet_search_cmd("saga-handler"), {
      stdout = "[]\n",
      stderr = "",
      exit_code = 0,
    })
    t.mock_command('printf %s "$FKST_GITHUB_WRITE"', { stdout = "1", stderr = "", exit_code = 0 })
    t.mock_command("gh issue close 979 --repo owner/repo", { stdout = "closed\n", stderr = "", exit_code = 0 })
    t.mock_command("pwd", { stdout = "/repo\n", stderr = "", exit_code = 0 })
    t.mock_command("python3 -B scripts/ratchet_migration_slicer.py code-dedup --repo-root /repo --slice-size 3 --json", {
      stdout = slice_json("code-dedup", { fingerprint = "dedup-fp", parent_issue = 1002 }),
      stderr = "",
      exit_code = 0,
    })
    t.mock_command(ratchet_search_cmd("code-dedup"), {
      stdout = "[]\n",
      stderr = "",
      exit_code = 0,
    })

    local result = run_scan({
      FKST_GITHUB_REPO = "owner/repo",
      FKST_GITHUB_WRITE = "1",
      FKST_RUNTIME_ROOT = "/tmp/fkst-packages-test/ratchet-slicer",
    })

    t.eq(result.exit_code, 0)
    t.eq(count_raises(result.raises, "github-proxy.github_issue_create_request"), 1)
  end,
}
