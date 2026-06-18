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

local function trusted_author(record, login)
  if login == nil or login == "" then
    return true
  end
  local author = record and (record.author_login or (type(record.author) == "table" and record.author.login))
  return core.strip_bot_login_suffix(author) == core.strip_bot_login_suffix(login)
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
    "python3",
    "scripts/ratchet_migration_slicer.py",
    ratchet.ratchet,
    "--json",
  }, 120)
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

local function parent_has_marker(parent, marker, login)
  for _, comment in ipairs(parent.comments or {}) do
    if trusted_author(comment, login) and body(comment):find(marker, 1, true) ~= nil then
      return true
    end
  end
  return false
end

local function parent_has_issue_created_marker(parent, dedup_key, login)
  for _, comment in ipairs(parent.comments or {}) do
    if trusted_author(comment, login) then
      for marker in body(comment):gmatch("<!%-%- fkst:github%-proxy:issue%-created:v1.-%-%->") do
        if marker:match('dedup="([^"]+)"') == dedup_key then
          return true
        end
      end
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

local function has_open_slice(github, repo, ratchet, login)
  for _, issue in ipairs(search_issues(github, repo, ratchet_slice_search_query(ratchet), "number,title,state,author,body,url", 30)) do
    if trusted_author(issue, login)
      and body(issue):find("fkst:ratchet-slice:v1", 1, true) ~= nil
      and body(issue):find('ratchet="' .. tostring(ratchet) .. '"', 1, true) ~= nil
      and tostring(issue.state or ""):upper() ~= "CLOSED" then
      return issue
    end
  end
  return nil
end

local function has_existing_slice(github, repo, dedup_key, login)
  local marker = issue_create_marker(dedup_key)
  for _, issue in ipairs(search_issues(github, repo, marker, "number,title,state,author,body,url", 30)) do
    if trusted_author(issue, login) and body(issue):find(marker, 1, true) ~= nil then
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
  return parse_created_issue_number(result and result.stdout)
end

local function parent_issue(github, repo, ratchet)
  local result = github.issue_view(repo, ratchet.parent_issue, "number,state,comments,author", 30)
  return decode_json_object(result and result.stdout or "{}", "parent issue")
end

local function reconcile_one(github, repo, ratchet)
  local login = bot_login()
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
  if parent_has_issue_created_marker(parent, dedup_key, login) then
    return "deduped-parent-ledger"
  end
  if has_open_slice(github, repo, ratchet.ratchet, login) ~= nil then
    return "deduped-in-flight"
  end
  if has_existing_slice(github, repo, dedup_key, login) ~= nil then
    return "deduped-existing-slice"
  end
  if not write_enabled() then
    return "would-create-slice"
  end

  local intent = issue_create_intent_marker(dedup_key)
  if not parent_has_marker(parent, intent, login) then
    write_comment(github, repo, ratchet.parent_issue, dedup_key, "intent", intent .. "\n")
  end
  local issue_number = create_issue(github, repo, slice)
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
