local core = require("core")
local ports_seam = require("std.ports")
local ratchets = require("departments.ratchet_migration_driver.ratchets")
local saga = require("std.saga")

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
  local login = core._trim(read_env("FKST_GITHUB_BOT_LOGIN") or "")
  if write_enabled() and login == "" then
    error("github-devloop: FKST_GITHUB_BOT_LOGIN is required when FKST_GITHUB_WRITE=1")
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
    local login = core.strip_bot_login_suffix(core._trim(entry))
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

local function decode_json_object(stdout, context)
  local ok, decoded = pcall(json.decode, stdout or "")
  if not ok or type(decoded) ~= "table" then
    error("github-devloop: invalid ratchet migration " .. tostring(context) .. " JSON")
  end
  return decoded
end

local function decode_json_list(stdout)
  local ok, decoded = pcall(json.decode, stdout or "")
  if not ok or type(decoded) ~= "table" then
    error("github-devloop: invalid GitHub issue search JSON")
  end
  return decoded
end

local function plan_for(ratchet)
  local result = exec_argv({
    argv = {
      "python3",
      "scripts/ratchet_migration_slicer.py",
      ratchet.ratchet,
      "--json",
    },
    timeout = 120,
  })
  if type(result) ~= "table" or result.exit_code ~= 0 then
    error("github-devloop: ratchet migration slicer failed for " .. tostring(ratchet.ratchet))
  end
  local plan = decode_json_object(result.stdout, "plan")
  if plan.schema_version ~= "fkst.ratchet-slice.v1" then
    error("github-devloop: unsupported ratchet migration plan schema")
  end
  if plan.ratchet ~= ratchet.ratchet or plan.allowlist_path ~= ratchet.allowlist_path then
    error("github-devloop: ratchet migration plan/config mismatch")
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

local function parse_created_issue_number(stdout)
  local text = tostring(stdout or "")
  local number = text:match("/issues/(%d+)") or text:match("#(%d+)")
  return number
end

local function require_issue_number(issue_number, context)
  local number = tonumber(issue_number)
  if number == nil then
    error("github-devloop: missing issue number for " .. tostring(context))
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

local function parent_has_issue_created_marker(parent, dedup_key, trusted_logins)
  for _, comment in ipairs(parent.comments or {}) do
    if trusted_author(comment, trusted_logins) then
      for marker in body(comment):gmatch("<!%-%- fkst:github%-proxy:issue%-created:v1.-%-%->") do
        if marker:match('dedup="([^"]+)"') == dedup_key then
          return tonumber(marker:match('issue="(%d+)"')) or "unresolved"
        end
      end
    end
  end
  return nil
end

local function search_issues(github, repo, query, fields, timeout)
  local result = github.issue_search(repo, query, fields or "number,title,state,author,body,url", timeout or 30)
  if type(result) == "table" and result.stdout ~= nil then
    return decode_json_list(result.stdout)
  end
  return result or {}
end

local function has_open_slice(github, repo, ratchet, trusted_logins)
  local ratchet_name = type(ratchet) == "table" and ratchet.ratchet or ratchet
  local entry_blob = type(ratchet) == "table" and tostring(body(ratchet):match('entries="([^"]*)"') or "") or ""
  for _, issue in ipairs(search_issues(github, repo, ratchet_slice_search_query(ratchet_name), "number,title,state,author,body,url", 30)) do
    for marker in body(issue):gmatch("<!%-%- fkst:ratchet%-slice:v1.-%-%->") do
      local entries = marker:match('entries="([^"]*)"'); local overlap = entries == nil or entry_blob == ""
      if not overlap then tostring(entries):gsub("[^,]+", function(entry) if ("," .. entry_blob .. ","):find("," .. entry .. ",", 1, true) then overlap = true end end) end
      if trusted_author(issue, trusted_logins) and marker:find('ratchet="' .. tostring(ratchet_name) .. '"', 1, true) ~= nil and tostring(issue.state or ""):upper() ~= "CLOSED" and overlap then
        return issue
      end end
  end return nil end

local function has_existing_slice(github, repo, dedup_key, trusted_logins)
  local marker = issue_create_marker(dedup_key)
  for _, issue in ipairs(search_issues(github, repo, marker, "number,title,state,author,body,url", 30)) do
    if trusted_author(issue, trusted_logins) and body(issue):find(marker, 1, true) ~= nil and tostring(issue.state or ""):upper() ~= "CLOSED" then
      return issue
    end
  end
  return nil
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

local function reconcile_one(github, repo, ratchet)
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
    error("github-devloop: invalid ratchet migration plan status")
  end

  local slice = plan.next_slice
  local dedup_key = tostring(slice.dedup_key or "")
  local ledger_issue = parent_has_issue_created_marker(parent, dedup_key, trusted_logins)
  if ledger_issue ~= nil then local prior = ledger_issue ~= "unresolved" and decode_json_object((github.issue_view(repo, ledger_issue, "number,state,author,body", 30) or {}).stdout or "{}", "child issue") or nil; if prior == nil or not trusted_author(prior, trusted_logins) or tostring(prior.state or ""):upper() ~= "CLOSED" then return "deduped-parent-ledger" end end

  if has_open_slice(github, repo, { ratchet = ratchet.ratchet, body = slice.body }, trusted_logins) ~= nil then
    return "deduped-in-flight"
  end
  if has_existing_slice(github, repo, dedup_key, trusted_logins) ~= nil then
    return "deduped-existing-slice"
  end
  if not write_enabled() then
    return "would-create-slice"
  end

  local intent = issue_create_intent_marker(dedup_key)
  if not parent_has_marker(parent, intent, trusted_logins) then
    write_comment(github, repo, ratchet.parent_issue, dedup_key, "intent", intent .. "\n")
  end
  local issue_number = create_issue(github, repo, slice)
  github.issue_add_sub_issue(repo, ratchet.parent_issue, issue_number, 30)
  write_comment(github, repo, ratchet.parent_issue, dedup_key, "created", issue_created_marker(dedup_key, issue_number) .. "\n")
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
      error("github-devloop: FKST_GITHUB_REPO is required")
    end
    local selected = event and event.payload and event.payload.ratchet
    for _, ratchet in ipairs(ratchets) do
      if selected == nil or selected == ratchet.ratchet then
        local action = reconcile_one(ports.github, repo, ratchet)
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
