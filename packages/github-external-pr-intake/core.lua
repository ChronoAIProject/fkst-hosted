local env = require("workflow.env")
local error_facts = require("contract.error_facts")
local logging = require("workflow.logging")
local strings = require("contract.strings")
local forge_strings = require("forge.strings")

local M = {}


local allowed_env = {
  FKST_GITHUB_BOT_LOGIN = true,
  FKST_GITHUB_REPO = true,
  FKST_GITHUB_WRITE = true,
  FKST_DEVLOOP_MANAGED_BOT_LOGINS = true,
  FKST_EXTERNAL_PR_BRIDGE_MIN_AGE_SECONDS = true,
}

local function read_env_command(name)
  if not allowed_env[name] then
    error("github-external-pr-intake: env-not-allowed: " .. tostring(name))
  end
  return 'printf %s "$' .. name .. '"'
end

M.read_env_command = read_env_command
M.read_env = env.read_env(read_env_command)
M.strip_bot_login_suffix = forge_strings.strip_bot_login_suffix
M.trim = strings.trim
M.json_string = strings.json_string
M.sanitize_key = strings.sanitize_key
M.DEFAULT_EXTERNAL_PR_BRIDGE_MIN_AGE_SECONDS = 3 * 60 * 60

function M.write_enabled()
  return M.read_env("FKST_GITHUB_WRITE") == "1"
end

function M.required_repo()
  local repo = M.trim(M.read_env("FKST_GITHUB_REPO") or "")
  if repo == "" or forge_strings.split_repo(repo) == nil then
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

local function is_leap_year(year)
  return year % 4 == 0 and (year % 100 ~= 0 or year % 400 == 0)
end

local function days_in_month(year, month)
  if month == 2 then
    return is_leap_year(year) and 29 or 28
  end
  if month == 4 or month == 6 or month == 9 or month == 11 then
    return 30
  end
  return 31
end

function M.iso_timestamp_epoch_seconds(timestamp)
  local year, month, day, hour, minute, second = tostring(timestamp or ""):match(
    "^(%d%d%d%d)%-(%d%d)%-(%d%d)T(%d%d):(%d%d):(%d%d)Z$"
  )
  if year == nil then
    year, month, day, hour, minute, second = tostring(timestamp or ""):match(
      "^(%d%d%d%d)%-(%d%d)%-(%d%d)T(%d%d):(%d%d):(%d%d)%.%d+Z$"
    )
  end
  if year == nil then
    return nil
  end
  year = tonumber(year)
  month = tonumber(month)
  day = tonumber(day)
  hour = tonumber(hour)
  minute = tonumber(minute)
  second = tonumber(second)
  if month < 1
    or month > 12
    or day < 1
    or day > days_in_month(year, month)
    or hour > 23
    or minute > 59
    or second > 59 then
    return nil
  end

  local adjusted_year = year
  local adjusted_month = month
  if adjusted_month <= 2 then
    adjusted_year = adjusted_year - 1
    adjusted_month = adjusted_month + 12
  end
  local era = math.floor(adjusted_year / 400)
  local year_of_era = adjusted_year - era * 400
  local day_of_year = math.floor((153 * (adjusted_month - 3) + 2) / 5) + day - 1
  local day_of_era = year_of_era * 365
    + math.floor(year_of_era / 4)
    - math.floor(year_of_era / 100)
    + day_of_year
  local days_since_epoch = era * 146097 + day_of_era - 719468
  return days_since_epoch * 86400 + hour * 3600 + minute * 60 + second
end

function M.external_pr_bridge_min_age_seconds()
  local raw = M.trim(M.read_env("FKST_EXTERNAL_PR_BRIDGE_MIN_AGE_SECONDS") or "")
  if raw == "" or raw:match("^%d+$") == nil then
    return M.DEFAULT_EXTERNAL_PR_BRIDGE_MIN_AGE_SECONDS
  end
  local parsed = tonumber(raw)
  if parsed == nil or parsed <= 0 then
    return M.DEFAULT_EXTERNAL_PR_BRIDGE_MIN_AGE_SECONDS
  end
  return parsed
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

function M.handled_marker(repo, pr_number, issue_number)
  return '<!-- fkst:github-external-pr-intake:external-pr-handled:v1 repo="'
    .. tostring(repo)
    .. '" pr="'
    .. tostring(M.safe_number(pr_number, "handled marker pr"))
    .. '" issue="'
    .. tostring(M.safe_number(issue_number, "handled marker issue"))
    .. '" -->'
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

local function label_names(record)
  local labels = {}
  for _, label in ipairs(record.labels or {}) do
    if type(label) == "table" and label.name ~= nil then
      table.insert(labels, tostring(label.name))
    elseif type(label) == "string" then
      table.insert(labels, label)
    end
  end
  return labels
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
    created_at = pr.createdAt or pr.created_at,
    updated_at = pr.updatedAt or pr.updated_at,
    author_login = author_login(pr),
    head_ref_name = head,
    base_ref_name = base,
    comments = comments(pr),
    assignees = assignee_logins(pr),
  }
end

function M.normalize_issue(issue)
  assert(type(issue) == "table", "normalize_issue requires a table")
  local state = tostring(issue.state or "")
  if state ~= "" then
    state = state:upper()
  end
  return {
    number = tonumber(issue.number),
    title = tostring(issue.title or ""),
    state = state,
    url = issue.url or issue.html_url,
    labels = label_names(issue),
    comments = comments(issue),
    author_login = author_login(issue),
  }
end

function M.is_external_candidate(pr, managed, now_seconds)
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
  local created_seconds = M.iso_timestamp_epoch_seconds(pr.created_at)
  local now_value = tonumber(now_seconds)
  if created_seconds == nil or now_value == nil then
    return false
  end
  return now_value - created_seconds >= M.external_pr_bridge_min_age_seconds()
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

local function marker_attr(marker, name)
  return tostring(marker or ""):match(tostring(name) .. '="([^"]*)"')
end

function M.bridge_issue_proposal_id(repo, issue_number)
  return "github-devloop/issue/" .. tostring(repo) .. "/" .. tostring(M.safe_number(issue_number, "proposal issue"))
end

function M.find_bridge_issue_merged_signal(issue, repo, issue_number, managed)
  local expected_proposal = M.bridge_issue_proposal_id(repo, issue_number)
  local signal = nil
  for _, comment in ipairs((issue and issue.comments) or {}) do
    if M.trusted_author(comment, managed) then
      local body = tostring(comment.body or "")
      for marker in body:gmatch("<!%-%- fkst:github%-devloop:merged:v1.-%-%->") do
        if marker_attr(marker, "proposal") == expected_proposal then
          signal = {
            proposal_id = expected_proposal,
            pr_number = tonumber(marker_attr(marker, "pr")),
            source = "merged-marker",
          }
        end
      end
      for marker in body:gmatch("<!%-%- fkst:github%-devloop:state:v1.-%-%->") do
        if marker_attr(marker, "proposal") == expected_proposal and marker_attr(marker, "state") == "merged" then
          signal = signal or {
            proposal_id = expected_proposal,
            source = "state-marker",
          }
        end
      end
    end
  end
  return signal
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

function M.find_pr_handled_marker(comments, repo, pr_number, issue_number, managed)
  local expected = M.handled_marker(repo, pr_number, issue_number)
  for _, comment in ipairs(comments or {}) do
    if M.trusted_author(comment, managed) and tostring(comment.body or ""):find(expected, 1, true) ~= nil then
      return {
        issue_number = M.safe_number(issue_number, "handled marker issue"),
        source = "handled-marker",
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
    "- MUST comply with project conventions (CLAUDE.md): file <= 1000 lines; source-internal text English; all gh/git via forge.github/forge.git adapters; saga-shaped departments; `scripts/run.sh test` green; ports/adapters; no compat/legacy shim; outward text English.",
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

function M.issue_url(repo, issue_number, issue)
  if type(issue) == "table" and issue.url ~= nil and tostring(issue.url) ~= "" then
    return tostring(issue.url)
  end
  return "https://github.com/" .. tostring(repo) .. "/issues/" .. tostring(M.safe_number(issue_number, "issue url"))
end

function M.pr_url(repo, pr_number)
  return "https://github.com/" .. tostring(repo) .. "/pull/" .. tostring(M.safe_number(pr_number, "pr url"))
end

function M.handled_comment_body(repo, pr, issue, signal)
  local pr_number = M.safe_number(pr.number, "handled comment pr")
  local issue_number = M.safe_number(issue.number, "handled comment issue")
  local lines = {
    "Thanks for the contribution. This change has been handled internally.",
    "",
    "Internal resolution:",
    "- Issue: " .. M.issue_url(repo, issue_number, issue),
  }
  if signal ~= nil and signal.pr_number ~= nil then
    table.insert(lines, "- PR: " .. M.pr_url(repo, signal.pr_number))
  end
  table.insert(lines, "")
  table.insert(lines, "Closing this external PR so the queue reflects that the content is already resolved.")
  table.insert(lines, "")
  table.insert(lines, M.handled_marker(repo, pr_number, issue_number))
  return table.concat(lines, "\n")
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
