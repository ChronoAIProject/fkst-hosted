local env = require("std.env")
local error_facts = require("std.error_facts")
local logging = require("std.logging")
local strings = require("std.strings")

local M = {}

function M.persistence_class()
  return "stateless_adapter"
end

local allowed_env = {
  FKST_GITHUB_BOT_LOGIN = true,
  FKST_GITHUB_REPO = true,
  FKST_GITHUB_WRITE = true,
  FKST_DEVLOOP_MANAGED_BOT_LOGINS = true,
}

local function read_env_command(name)
  if not allowed_env[name] then
    error("github-external-pr-intake: env-not-allowed: " .. tostring(name))
  end
  return 'printf %s "$' .. name .. '"'
end

M.read_env_command = read_env_command
M.read_env = env.read_env(read_env_command)
M.strip_bot_login_suffix = strings.strip_bot_login_suffix
M.trim = strings.trim
M.json_string = strings.json_string
M.sanitize_key = strings.sanitize_key

function M.write_enabled()
  return M.read_env("FKST_GITHUB_WRITE") == "1"
end

function M.required_repo()
  local repo = M.trim(M.read_env("FKST_GITHUB_REPO") or "")
  if repo == "" or strings.split_repo(repo) == nil then
    error("github-external-pr-intake: repo-required: FKST_GITHUB_REPO is required")
  end
  return repo
end

function M.current_bot_login()
  local login = M.strip_bot_login_suffix(M.trim(M.read_env("FKST_GITHUB_BOT_LOGIN") or ""))
  if M.write_enabled() and login == "" then
    error("github-external-pr-intake: bot-login-required: FKST_GITHUB_BOT_LOGIN is required when FKST_GITHUB_WRITE=1")
  end
  return login
end

function M.managed_bot_logins()
  local logins = {}
  local current = M.current_bot_login()
  if current ~= nil and current ~= "" then
    logins[current] = true
  end
  for entry in tostring(M.read_env("FKST_DEVLOOP_MANAGED_BOT_LOGINS") or ""):gmatch("[^,%s]+") do
    local login = M.strip_bot_login_suffix(M.trim(entry))
    if login ~= nil and login ~= "" then
      logins[login] = true
    end
  end
  return logins
end

function M.is_managed_bot_login(login, managed)
  local normalized = M.strip_bot_login_suffix(login)
  return normalized ~= nil and normalized ~= "" and managed[normalized] == true
end

function M.trusted_author(record, managed)
  local author = nil
  if type(record) == "table" then
    author = record.author_login
    if author == nil and type(record.author) == "table" then
      author = record.author.login
    end
    if author == nil and type(record.user) == "table" then
      author = record.user.login
    end
  end
  return M.is_managed_bot_login(author, managed)
end

function M.safe_number(value, context)
  local number = tonumber(value)
  if number == nil or number < 1 or number % 1 ~= 0 then
    error("github-external-pr-intake: invalid-number: " .. tostring(context))
  end
  return number
end

function M.source_ref(repo, pr_number)
  return {
    kind = "external",
    ref = tostring(repo) .. "#pr/" .. tostring(M.safe_number(pr_number, "pr")),
  }
end

function M.parse_source_ref(source_ref)
  if type(source_ref) ~= "table" or source_ref.kind ~= "external" then
    error("github-external-pr-intake: source-ref-required: external PR source_ref is required")
  end
  local repo, number = tostring(source_ref.ref or ""):match("^([^#]+)#pr/(%d+)$")
  if repo == nil then
    error("github-external-pr-intake: invalid-source-ref: external PR source_ref is required")
  end
  return repo, M.safe_number(number, "source_ref pr")
end

function M.bridge_marker(repo, pr_number, issue_number)
  local marker = '<!-- fkst:github-external-pr-intake:external-pr-bridge:v1 repo="'
    .. tostring(repo)
    .. '" pr="'
    .. tostring(M.safe_number(pr_number, "marker pr"))
    .. '" source_ref="external:'
    .. tostring(repo)
    .. "#pr/"
    .. tostring(pr_number)
    .. '"'
  if issue_number ~= nil then
    marker = marker .. ' issue="' .. tostring(M.safe_number(issue_number, "marker issue")) .. '"'
  end
  return marker .. " -->"
end

function M.bridge_search_query(repo, pr_number)
  return 'fkst:github-external-pr-intake:external-pr-bridge:v1 repo="'
    .. tostring(repo)
    .. '" pr="'
    .. tostring(M.safe_number(pr_number, "search pr"))
    .. '"'
end

function M.bridge_lock_key(repo, pr_number)
  return "github-external-pr-intake/bridge/"
    .. M.sanitize_key(tostring(repo), 140)
    .. "/pr/"
    .. tostring(M.safe_number(pr_number, "lock pr"))
end

function M.dedup_key(repo, pr_number)
  return "github-external-pr-intake/" .. tostring(repo) .. "/pr/" .. tostring(M.safe_number(pr_number, "dedup pr"))
end

function M.body_file_path(repo, pr_number, kind)
  local stem = M.sanitize_key(tostring(repo) .. "-pr-" .. tostring(pr_number), 160):gsub("/", "-")
  return "/tmp/fkst-github-external-pr-intake-"
    .. stem
    .. "-"
    .. tostring(kind or "body")
    .. ".md"
end

function M.parse_created_issue_number(stdout)
  local text = tostring(stdout or "")
  return tonumber(text:match("/issues/(%d+)") or text:match("#(%d+)"))
end

function M.decode_json_list(stdout, context)
  local ok, decoded = pcall(json.decode, stdout or "")
  if not ok or type(decoded) ~= "table" then
    error("github-external-pr-intake: invalid-json: " .. tostring(context))
  end
  return decoded
end

function M.decode_json_object(stdout, context)
  local ok, decoded = pcall(json.decode, stdout or "")
  if not ok or type(decoded) ~= "table" then
    error("github-external-pr-intake: invalid-json-object: " .. tostring(context))
  end
  return decoded
end

local function append_prs(target, value)
  if type(value) ~= "table" then
    return
  end
  if value.number ~= nil then
    table.insert(target, value)
    return
  end
  for _, item in ipairs(value) do
    append_prs(target, item)
  end
end

function M.parse_pr_list(stdout)
  local decoded = M.decode_json_list(stdout or "[]", "PR list")
  local prs = {}
  append_prs(prs, decoded)
  return prs
end

local function author_login(pr)
  if type(pr.author) == "table" then
    return pr.author.login
  end
  if type(pr.user) == "table" then
    return pr.user.login
  end
  if pr.author_login ~= nil then
    return pr.author_login
  end
  return nil
end

local function assignee_logins(pr)
  local logins = {}
  for _, assignee in ipairs(pr.assignees or {}) do
    if type(assignee) == "table" and assignee.login ~= nil then
      table.insert(logins, tostring(assignee.login))
    elseif type(assignee) == "string" then
      table.insert(logins, assignee)
    end
  end
  return logins
end

local function comments(pr)
  local result = {}
  for _, comment in ipairs(pr.comments or {}) do
    if type(comment) == "table" then
      local login = comment.author_login
      if login == nil and type(comment.author) == "table" then
        login = comment.author.login
      end
      if login == nil and type(comment.user) == "table" then
        login = comment.user.login
      end
      table.insert(result, {
        body = tostring(comment.body or ""),
        author_login = login,
        created_at = comment.createdAt or comment.created_at,
      })
    end
  end
  return result
end

function M.normalize_pr(pr, repo)
  assert(type(pr) == "table", "normalize_pr requires a table")
  local head = pr.headRefName or pr.head_ref_name
  if head == nil and type(pr.head) == "table" then
    head = pr.head.ref
  end
  local base = pr.baseRefName or pr.base_ref_name
  if base == nil and type(pr.base) == "table" then
    base = pr.base.ref
  end
  local state = tostring(pr.state or "")
  if state ~= "" then
    state = state:upper()
  end
  return {
    repo = repo,
    number = tonumber(pr.number),
    title = tostring(pr.title or ""),
    state = state,
    url = pr.url or pr.html_url,
    updated_at = pr.updatedAt or pr.updated_at,
    author_login = author_login(pr),
    head_ref_name = head,
    base_ref_name = base,
    comments = comments(pr),
    assignees = assignee_logins(pr),
  }
end

function M.is_external_candidate(pr, managed)
  if type(pr) ~= "table" or pr.number == nil then
    return false
  end
  if tostring(pr.state or "") ~= "" and tostring(pr.state):upper() ~= "OPEN" then
    return false
  end
  if M.is_managed_bot_login(pr.author_login, managed) then
    return false
  end
  if tostring(pr.head_ref_name or ""):match("^devloop/") ~= nil then
    return false
  end
  return true
end

function M.bridge_marker_issue_number(body)
  for marker in tostring(body or ""):gmatch("<!%-%- fkst:github%-external%-pr%-intake:external%-pr%-bridge:v1.-%-%->") do
    local issue = tonumber(marker:match('issue="(%d+)"'))
    if issue ~= nil then
      return issue
    end
  end
  return nil
end

function M.find_pr_bridge_marker(comments, repo, pr_number, managed)
  local expected = M.bridge_search_query(repo, pr_number)
  for _, comment in ipairs(comments or {}) do
    if M.trusted_author(comment, managed) and tostring(comment.body or ""):find(expected, 1, true) ~= nil then
      return {
        issue_number = M.bridge_marker_issue_number(comment.body),
        source = "pr-marker",
      }
    end
  end
  return nil
end

function M.bridge_issue_body(repo, pr)
  local number = M.safe_number(pr.number, "issue body pr")
  local source = "external:" .. tostring(repo) .. "#pr/" .. tostring(number)
  return table.concat({
    M.bridge_marker(repo, number),
    "",
    "- Source: external PR #" .. tostring(number) .. " (refs/pull/" .. tostring(number) .. "/head), author @"
      .. tostring(pr.author_login or "unknown") .. ". source_ref: " .. source,
    "- Task: implement/complete the change BASED ON the existing code in PR #" .. tostring(number)
      .. " - fetch `refs/pull/" .. tostring(number)
      .. "/head`, build ON the contributor's work, do NOT rewrite from scratch. Re-derive the full diff from source_ref.",
    "- MUST comply with project conventions (CLAUDE.md): file <= 1000 lines; source-internal text English; all gh/git via std.github/std.git adapters; saga-shaped departments; `scripts/run.sh test` green; ports/adapters; no compat/legacy shim; outward text English.",
    "- If PR #" .. tostring(number) .. "'s base is not a managed branch (current base: `"
      .. tostring(pr.base_ref_name or "") .. "`), implement against `dev`.",
    "- On completion, the resulting devloop PR supersedes external PR #" .. tostring(number)
      .. "; close #" .. tostring(number) .. " with a link to this issue and the devloop PR.",
    "",
  }, "\n")
end

function M.bridge_issue_title(pr)
  return "Integrate external PR #" .. tostring(M.safe_number(pr.number, "issue title pr")) .. ": " .. tostring(pr.title or "")
end

M.error_fingerprint = error_facts.error_fingerprint

function M.error_class_from_message(message)
  local text = tostring(message or "")
  return text:match("github%-external%-pr%-intake: ([%w%-]+):") or "caught-failure"
end

function M.log_line(level, dept, proposal_id, tag, fields)
  return logging.log_line("github-external-pr-intake", level, dept, proposal_id, tag, fields)
end

function M.log_entry(dept, event, proposal_id, dedup_key)
  return logging.log_entry("github-external-pr-intake", dept, event, proposal_id, dedup_key)
end

function M.log_error_fact(level, dept, proposal_id, tag, error_class, queue, message, context)
  local fields = error_facts.error_fact_fields(error_class, queue, dept, message, context)
  table.insert(fields, "queue=" .. error_facts.one_line(queue))
  table.insert(fields, "error=" .. error_facts.one_line(message))
  M.log_line(level or "error", dept, proposal_id, tag or "FAILURE", fields)
end

function M.wrap_pipeline_failure(dept, fn)
  return function(event)
    local ok, result = pcall(fn, event)
    if ok then
      return result
    end
    M.log_error_fact("error", dept, "external-pr-intake", "FAILURE", M.error_class_from_message(result), type(event) == "table" and event.queue or nil, result, {
      source_ref = error_facts.event_source_ref(event),
      attempt = type(event) == "table" and event.attempt or nil,
    })
    error(result, 0)
  end
end

return M
