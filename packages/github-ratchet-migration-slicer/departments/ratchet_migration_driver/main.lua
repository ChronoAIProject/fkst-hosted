local core = require("core")
local ports_seam = require("forge.ports")
local ratchets = require("departments.ratchet_migration_driver.ratchets")
local saga = require("workflow.saga")
local strings = require("contract.strings")

local M = {}

local spec = {
  consumes = { "ratchet_migration_poll" },
  produces = {},
  ephemeral = { "ratchet_migration_poll" },
  retry = false,
  stall_window = "2m",
}

local function read_env(name)
  return core.read_env(name)
end

local function write_enabled()
  return read_env("FKST_GITHUB_WRITE") == "1"
end

local function bot_login()
  local login = strings.trim(read_env("FKST_GITHUB_BOT_LOGIN") or "")
  if write_enabled() and login == "" then
    error("github-ratchet-migration-slicer: bot-login-required: FKST_GITHUB_BOT_LOGIN is required when FKST_GITHUB_WRITE=1")
  end
  return login
end

local function trusted_bot_logins()
  local logins = {}
  local current = core.strip_bot_login_suffix(bot_login())
  if current == nil or current == "" then
    return logins
  end
  logins[current] = true
  for entry in tostring(read_env("FKST_DEVLOOP_MANAGED_BOT_LOGINS") or ""):gmatch("[^,%s]+") do
    local login = core.strip_bot_login_suffix(strings.trim(entry))
    if login ~= nil and login ~= "" then
      logins[login] = true
    end
  end
  return logins
end

local function set_empty(values)
  for _ in pairs(values or {}) do
    return false
  end
  return true
end

local function trusted_author(record, trusted_logins)
  if set_empty(trusted_logins) then
    return true
  end
  local author = record and (record.author_login or (type(record.author) == "table" and record.author.login))
  author = core.strip_bot_login_suffix(author)
  return author ~= nil and trusted_logins[author] == true
end

local function body(record)
  return tostring(record and record.body or "")
end

local function json_string(value)
  return strings.json_string(value)
end

local function decode_json_object(stdout, context)
  local ok, decoded = pcall(json.decode, stdout or "")
  if not ok or type(decoded) ~= "table" then
    error("github-ratchet-migration-slicer: invalid-ratchet-json: " .. tostring(context))
  end
  return decoded
end

local function decode_json_list(stdout)
  local ok, decoded = pcall(json.decode, stdout or "")
  if not ok or type(decoded) ~= "table" then
    error("github-ratchet-migration-slicer: invalid-issue-search-json: GitHub issue search")
  end
  return decoded
end

local function debug_source_path(level)
  if type(debug) ~= "table" or type(debug.getinfo) ~= "function" then
    return nil
  end
  local info = debug.getinfo(level or 1, "S")
  local source = info and info.source or ""
  if source:sub(1, 1) == "@" then
    return source:sub(2)
  end
  return nil
end

local function strip_suffix(text, suffix)
  text = tostring(text or "")
  suffix = tostring(suffix or "")
  if suffix ~= "" and text:sub(-#suffix) == suffix then
    return text:sub(1, #text - #suffix)
  end
  return nil
end

local function package_root()
  return strip_suffix(debug_source_path(1), "/departments/ratchet_migration_driver/main.lua")
    or "packages/github-ratchet-migration-slicer"
end

local function slicer_tool_path()
  return package_root() .. "/tools/ratchet_migration_slicer.py"
end

local function allowlist_exists(ratchet)
  local path = tostring(ratchet and ratchet.allowlist_path or "")
  if path == "" then
    return false
  end
  if type(file) == "table" and type(file.exists) == "function" then
    return file.exists(path) == true
  end
  local ok = pcall(file.read, path)
  return ok == true
end

local function plan_for(ratchet)
  local result = exec_argv({
    argv = {
      "python3",
      slicer_tool_path(),
      ratchet.ratchet,
      "--repo-root",
      ".",
      "--json",
    },
    timeout = 120,
  })
  if type(result) ~= "table" or result.exit_code ~= 0 then
    error("github-ratchet-migration-slicer: plan-command-failed: " .. tostring(ratchet.ratchet))
  end
  local plan = decode_json_object(result.stdout, "plan")
  if plan.schema_version ~= "fkst.ratchet-slice.v1" then
    error("github-ratchet-migration-slicer: unsupported-plan-schema: " .. tostring(plan.schema_version))
  end
  if plan.ratchet ~= ratchet.ratchet or plan.allowlist_path ~= ratchet.allowlist_path then
    error("github-ratchet-migration-slicer: plan-config-mismatch: " .. tostring(ratchet.ratchet))
  end
  return plan
end

local function safe_runtime_segment(value)
  local safe = tostring(value or ""):gsub("[^%w._-]", "_")
  safe = safe:gsub("_+", "_"):gsub("^_+", ""):gsub("_+$", "")
  if safe == "" then
    return "empty"
  end
  return safe:sub(1, 160)
end

local function body_file(dedup_key, kind)
  return "/tmp/fkst-github-devloop-ratchet-" .. safe_runtime_segment(dedup_key) .. "-" .. tostring(kind) .. ".md"
end

local function timestamp_now()
  return os.date("!%Y-%m-%dT%H:%M:%SZ", tonumber(now()) or os.time())
end

local function issue_create_marker(dedup_key)
  return "<!-- fkst:github-proxy:issue-create:" .. tostring(dedup_key) .. " -->"
end

local function issue_create_intent_marker(dedup_key)
  return '<!-- fkst:github-proxy:issue-create-intent:v1 dedup="' .. tostring(dedup_key) .. '" -->'
end

local function issue_created_marker(dedup_key, issue_number)
  return '<!-- fkst:github-proxy:issue-created:v1 dedup="' .. tostring(dedup_key)
    .. '" issue="' .. tostring(issue_number or "unknown") .. '" -->'
end

local function ratchet_slice_search_query(ratchet)
  return 'fkst:ratchet-slice:v1 ratchet="' .. tostring(ratchet) .. '"'
end

local function require_entry_key(slice)
  if type(slice) ~= "table" or type(slice.sites) ~= "table" or type(slice.sites[1]) ~= "table" then
    error("github-ratchet-migration-slicer: missing-entry-key: ratchet migration slice is missing entry_key")
  end
  local entry_key = tostring(slice.sites[1].entry_key or "")
  if not entry_key:match("^[0-9a-f]+$") or #entry_key ~= 64 then
    error("github-ratchet-migration-slicer: invalid-entry-key: invalid ratchet migration entry_key")
  end
  return entry_key
end

local function parse_created_issue_number(stdout)
  local text = tostring(stdout or "")
  local number = text:match("/issues/(%d+)") or text:match("#(%d+)")
  return number
end

local function require_issue_number(issue_number, context)
  local number = tonumber(issue_number)
  if number == nil then
    error("github-ratchet-migration-slicer: missing-issue-number: " .. tostring(context))
  end
  return number
end

local function parent_has_marker(parent, marker, trusted_logins)
  for _, comment in ipairs(parent.comments or {}) do
    if trusted_author(comment, trusted_logins) and body(comment):find(marker, 1, true) ~= nil then
      return true
    end
  end
  return false
end

local function search_issues(github, repo, query, fields, timeout)
  local result = github.issue_search(repo, query, fields or "number,title,state,author,body,url", timeout or 30)
  if type(result) == "table" and result.stdout ~= nil then
    return decode_json_list(result.stdout)
  end
  return result or {}
end

local function marker_attr(marker, name)
  return marker:match('%f[%w_]' .. name .. '="([^"]*)"')
end

local function marker_entry_key(marker)
  local key = marker_attr(marker, "entry_key")
  if key ~= nil and key ~= "" then
    return key
  end
  local entries = marker_attr(marker, "entries")
  if entries ~= nil and entries ~= "" and not entries:find(",", 1, true) then
    return entries
  end
  return nil
end

local function slice_issue_entry_key(issue, ratchet_name)
  for marker in body(issue):gmatch("<!%-%- fkst:ratchet%-slice:v1.-%-%->") do
    if marker_attr(marker, "ratchet") == tostring(ratchet_name) then
      local key = marker_entry_key(marker)
      if key ~= nil then
        return key
      end
    end
  end
  return nil
end

local function find_open_slice_by_entry_key(github, repo, ratchet_name, entry_key)
  for _, issue in ipairs(search_issues(github, repo, ratchet_slice_search_query(ratchet_name), "number,title,state,author,body,url", 30)) do
    if tostring(issue.state or ""):upper() ~= "CLOSED"
      and slice_issue_entry_key(issue, ratchet_name) == tostring(entry_key) then
      return issue
    end
  end
  return nil
end

local function read_ledger(git, entry_key)
  local ref = core.ratchet_slice_ledger_ref(entry_key)
  local listed = git.ls_remote_ref("origin", ref, 30)
  if type(listed) ~= "table" or listed.exit_code ~= 0 then
    error("github-ratchet-migration-slicer: ledger-ls-remote-failed: git ledger ls-remote failed: " .. tostring(listed and listed.stderr or "missing result"))
  end
  local sha = core.parse_ratchet_slice_ledger_ref_sha(listed.stdout)
  if sha == nil then
    return nil
  end
  local fetched = git.fetch_ref("origin", ref, 30)
  if type(fetched) ~= "table" or fetched.exit_code ~= 0 then
    error("github-ratchet-migration-slicer: ledger-fetch-failed: git ledger fetch failed: " .. tostring(fetched and fetched.stderr or "missing result"))
  end
  local commit = git.cat_file_pretty(sha, 30)
  if type(commit) ~= "table" or commit.exit_code ~= 0 then
    error("github-ratchet-migration-slicer: ledger-cat-file-failed: git ledger cat-file failed: " .. tostring(commit and commit.stderr or "missing result"))
  end
  return {
    ref = ref,
    sha = sha,
    data = core.decode_ratchet_slice_ledger(commit.stdout),
  }
end

local function ledger_issue_is_open(github, repo, ledger)
  local number = ledger and ledger.data and tonumber(ledger.data.issue_number)
  if number == nil then
    return false
  end
  local issue = decode_json_object((github.issue_view(repo, number, "number,state,author,body", 30) or {}).stdout or "{}", "ledger issue")
  return tostring(issue.state or ""):upper() ~= "CLOSED", issue
end

local function ledger_records_issue(ledger)
  return type(ledger) == "table"
    and type(ledger.data) == "table"
    and ledger.data.state == "issue-created"
    and tonumber(ledger.data.issue_number) ~= nil
end

local function ledger_claim_is_fresh(ledger)
  if ledger == nil or type(ledger.data) ~= "table" or ledger.data.state ~= "claiming" then
    return false
  end
  local seconds = core.iso_timestamp_epoch_seconds(ledger.data.claimed_at)
  local current = tonumber(now())
  if seconds == nil or current == nil then
    return false
  end
  return current - seconds >= 0 and current - seconds <= 30 * 60
end

local function ledger_generation(ledger)
  local value = ledger and ledger.data and tonumber(ledger.data.generation)
  if value == nil or value < 1 then
    return 0
  end
  return value
end

local function ledger_json(state)
  return "{"
    .. '"schema":"fkst.ratchet-migration-slice-ledger.v1"'
    .. ',"state":' .. json_string(state.state)
    .. ',"entry_key":' .. json_string(state.entry_key)
    .. ',"allowlist_path":' .. json_string(state.allowlist_path)
    .. ',"generation":' .. tostring(tonumber(state.generation) or 1)
    .. ',"claim_owner":' .. json_string(state.claim_owner or "")
    .. ',"claimed_at":' .. json_string(state.claimed_at or "")
    .. ',"issue_number":' .. (state.issue_number ~= nil and tostring(tonumber(state.issue_number) or 0) or "null")
    .. ',"updated_at":' .. json_string(state.updated_at or timestamp_now())
    .. "}"
end

local function commit_ledger(git, ledger, state, dedup_key)
  local head_tree = git.rev_parse_ref_tree("HEAD", 30)
  if type(head_tree) ~= "table" or head_tree.exit_code ~= 0 then
    error("github-ratchet-migration-slicer: ledger-head-tree-failed: git ledger HEAD tree failed: " .. tostring(head_tree and head_tree.stderr or "missing result"))
  end
  local tree_sha = tostring(head_tree.stdout or ""):match("(%x+)")
  if tree_sha == nil or #tree_sha ~= 40 then
    error("github-ratchet-migration-slicer: ledger-unsafe-tree-sha: git ledger unsafe tree sha")
  end
  local path = body_file(dedup_key, "ledger")
  file.write(path, ledger_json(state) .. "\n")
  local commit = git.commit_tree(tree_sha, ledger and ledger.sha or nil, path, 30)
  if type(commit) ~= "table" or commit.exit_code ~= 0 then
    error("github-ratchet-migration-slicer: ledger-commit-tree-failed: git ledger commit-tree failed: " .. tostring(commit and commit.stderr or "missing result"))
  end
  local sha = tostring(commit.stdout or ""):match("(%x+)")
  if sha == nil or #sha ~= 40 then
    error("github-ratchet-migration-slicer: ledger-unsafe-commit-sha: git ledger unsafe commit sha")
  end
  return sha
end

local function write_ledger_state(git, ledger, state, dedup_key)
  local sha = commit_ledger(git, ledger, state, dedup_key)
  local pushed = git.push_ref_update("origin", sha, core.ratchet_slice_ledger_ref(state.entry_key), false, 60)
  if type(pushed) ~= "table" or pushed.exit_code ~= 0 then
    return nil, pushed
  end
  return sha, pushed
end

local function acquire_claim(git, ledger, slice, entry_key)
  local generation = ledger_generation(ledger) + 1
  local state = {
    state = "claiming",
    entry_key = entry_key,
    allowlist_path = tostring(slice.allowlist_path or ""),
    generation = generation,
    claim_owner = bot_login(),
    claimed_at = timestamp_now(),
    updated_at = timestamp_now(),
  }
  local sha, pushed = write_ledger_state(git, ledger, state, slice.dedup_key)
  if sha == nil then
    return nil, pushed
  end
  return {
    ref = core.ratchet_slice_ledger_ref(entry_key),
    sha = sha,
    data = state,
  }
end

local function mark_issue_created(git, ledger, slice, entry_key, issue_number)
  local state = {
    state = "issue-created",
    entry_key = entry_key,
    allowlist_path = tostring(slice.allowlist_path or ""),
    generation = ledger_generation(ledger),
    claim_owner = ledger and ledger.data and ledger.data.claim_owner or bot_login(),
    claimed_at = ledger and ledger.data and ledger.data.claimed_at or "",
    issue_number = tonumber(issue_number),
    updated_at = timestamp_now(),
  }
  return write_ledger_state(git, ledger, state, slice.dedup_key)
end

local function adopt_open_slice(git, ledger, slice, entry_key, issue)
  local number = require_issue_number(issue and issue.number, "adopted ratchet slice")
  if ledger_records_issue(ledger) and tonumber(ledger.data.issue_number) == number then
    return
  end
  local base = ledger
  if base ~= nil and ledger_claim_is_fresh(base) then
    return
  end
  local generation = ledger_generation(base)
  if generation < 1 then
    generation = 1
  end
  local state = {
    state = "issue-created",
    entry_key = entry_key,
    allowlist_path = tostring(slice.allowlist_path or ""),
    generation = generation,
    claim_owner = "adopted-open-issue",
    claimed_at = "",
    issue_number = number,
    updated_at = timestamp_now(),
  }
  write_ledger_state(git, base, state, slice.dedup_key)
end

local function write_comment(github, repo, issue_number, dedup_key, kind, text)
  local path = body_file(dedup_key, kind)
  file.write(path, text)
  return github.issue_comment(repo, issue_number, path, 30)
end

local function create_issue(github, repo, slice)
  local path = body_file(slice.dedup_key, "body")
  file.write(path, tostring(slice.body or ""))
  local result = github.issue_create(repo, slice.title, path, slice.labels or { "fkst-dev:enabled" }, {}, 30)
  return require_issue_number(parse_created_issue_number(result and result.stdout), "created ratchet slice")
end

local function parent_issue(github, repo, ratchet)
  local result = github.issue_view(repo, ratchet.parent_issue, "number,state,comments,author", 30)
  return decode_json_object(result and result.stdout or "{}", "parent issue")
end

local function reconcile_one(github, git, repo, ratchet)
  if not allowlist_exists(ratchet) then
    return "not-applicable-here"
  end
  local trusted_logins = trusted_bot_logins()
  local plan = plan_for(ratchet)
  local parent = parent_issue(github, repo, ratchet)
  if plan.status == "inventory_empty" then
    if tostring(parent.state or ""):upper() ~= "OPEN" then
      return "parent-already-closed"
    end
    if write_enabled() then
      github.issue_close(repo, ratchet.parent_issue, 30)
      return "closed-parent"
    end
    return "would-close-parent"
  end
  if plan.status ~= "slice_available" or type(plan.next_slice) ~= "table" then
    error("github-ratchet-migration-slicer: invalid-plan-status: " .. tostring(plan.status))
  end

  local slice = plan.next_slice
  slice.allowlist_path = plan.allowlist_path
  local dedup_key = tostring(slice.dedup_key or "")
  local entry_key = require_entry_key(slice)
  local discovered = find_open_slice_by_entry_key(github, repo, ratchet.ratchet, entry_key)
  local ledger = read_ledger(git, entry_key)
  if discovered ~= nil then
    if write_enabled() then
      adopt_open_slice(git, ledger, slice, entry_key, discovered)
    end
    return "deduped-entry-key"
  end
  local ledger_open, ledger_issue = ledger_issue_is_open(github, repo, ledger)
  if ledger_open then
    return "deduped-ref-ledger"
  end
  if ledger_claim_is_fresh(ledger) then
    return "deduped-ref-claim"
  end
  if not write_enabled() then
    return "would-create-slice"
  end

  local claim = acquire_claim(git, ledger, slice, entry_key)
  if claim == nil then
    local winner = find_open_slice_by_entry_key(github, repo, ratchet.ratchet, entry_key)
    if winner ~= nil then
      return "deduped-entry-key"
    end
    return "deduped-ref-race"
  end

  local intent = issue_create_intent_marker(dedup_key)
  if not parent_has_marker(parent, intent, trusted_logins) then
    write_comment(github, repo, ratchet.parent_issue, dedup_key, "intent", intent .. "\n")
  end
  local issue_number = create_issue(github, repo, slice)
  github.issue_add_sub_issue(repo, ratchet.parent_issue, issue_number, 30)
  write_comment(github, repo, ratchet.parent_issue, dedup_key, "created", issue_created_marker(dedup_key, issue_number) .. "\n")
  mark_issue_created(git, claim, slice, entry_key, issue_number)
  return "created-slice"
end

local function make_department(ports)
  local function done(_event)
    return false
  end

  local function act(event)
    core.log_entry("ratchet_migration_driver", event, "ratchet-migration", "poll")
    if event ~= nil and event.payload ~= nil and type(event.payload) ~= "table" then
      return
    end
    local repo = read_env("FKST_GITHUB_REPO")
    if repo == nil or repo == "" then
      error("github-ratchet-migration-slicer: repo-required: FKST_GITHUB_REPO is required")
    end
    local selected = event and event.payload and event.payload.ratchet
    for _, ratchet in ipairs(ratchets) do
      if selected == nil or selected == ratchet.ratchet then
        local action = reconcile_one(ports.github, ports.git, repo, ratchet)
        core.log_line("info", "ratchet_migration_driver", tostring(ratchet.ratchet), "ACTION", {
          "action=" .. tostring(action),
        })
      end
    end
  end

  local previous_pipeline = _G.pipeline
  local department = saga.department(spec, {
    done = done,
    act = act,
    wrap = core.wrap_pipeline_failure,
    name = "ratchet_migration_driver",
  })
  department.pipeline = _G.pipeline
  _G.pipeline = previous_pipeline
  return department
end

M = ports_seam.install(make_department)
M.make_department = make_department
_G.pipeline = M.pipeline

return M
