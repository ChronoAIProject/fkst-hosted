-- github-issue: label-scoped issue discovery.
--
-- The shared `list_scopes` body that was duplicated verbatim in
-- workflow-security/discovery.lua and workflow-writer/discovery.lua. It searches
-- the repo for OPEN issues carrying the adapter's declared label (via the
-- injected `github` port, never _G), fetches each issue's full text, and returns
-- the discovery scope handles the kernel reconcile expects. Every GitHub read is
-- defensive (pcall + validate) so a transient failure yields an empty list — a
-- safe wait — never a crash.
local label = require("github-issue.label")

local M = {}

local SEARCH_FIELDS = "number,title,state,author,body,url"
local VIEW_FIELDS = "number,title,state,body,comments,author,url"
local SEARCH_LIMIT = 30
local VIEW_TIMEOUT = 30

M.SEARCH_FIELDS = SEARCH_FIELDS
M.VIEW_FIELDS = VIEW_FIELDS
M.SEARCH_LIMIT = SEARCH_LIMIT
M.VIEW_TIMEOUT = VIEW_TIMEOUT

local function decode_json(text)
  local ok, decoded = pcall(json.decode, tostring(text or ""))
  if not ok then
    return nil
  end
  return decoded
end

M.decode_json = decode_json

local function open_state(value)
  return tostring(value or ""):upper() == "OPEN"
end

M.open_state = open_state

-- A GitHub App's author login has three forms across GitHub's surfaces: bare
-- "<slug>" via GraphQL (which `gh issue view --json comments` uses to populate
-- comment.author.login), "<slug>[bot]" via the REST API, and "app/<slug>" via
-- gh's `issue view --json author` for an App-authored ISSUE. Normalize both
-- sides -- lowercase, strip a leading "app/" and a trailing "[bot]" -- so bot
-- authorship matches the configured FKST_GITHUB_BOT_LOGIN regardless of which
-- surface populated the field. No-op for ordinary user logins (which never
-- carry either affix; a real username can never contain "/"). Mirrors the
-- shared forge.strings / devloop.base normalizer, inlined here to keep this a
-- leaf library with no lib_deps.
local function normalize_login(login)
  return (tostring(login or ""):lower():gsub("^app/", ""):gsub("%[bot%]$", ""))
end

M.normalize_login = normalize_login

-- Concatenate the issue body with every comment body the bot authored (or all
-- comments when no bot_login is pinned). This is the trusted marker-carrying text
-- the kernel parses adapter state out of.
local function trusted_text(issue, bot_login)
  local parts = { tostring(issue.body or "") }
  local comments = issue.comments
  if type(comments) == "table" then
    local trusted = normalize_login(bot_login)
    for _, comment in ipairs(comments) do
      if type(comment) == "table" then
        local login = type(comment.author) == "table" and comment.author.login or comment.author_login
        if bot_login == nil or bot_login == "" or normalize_login(login) == trusted then
          table.insert(parts, tostring(comment.body or ""))
        end
      end
    end
  end
  return table.concat(parts, "\n")
end

M.trusted_text = trusted_text

-- Fetch one issue's full view (body + comments), or nil on a transient failure.
local function fetch_issue(github, repo, number)
  if type(github) ~= "table" or type(github.issue_view) ~= "function" then
    return nil
  end
  local ok, result = pcall(github.issue_view, repo, number, VIEW_FIELDS, VIEW_TIMEOUT)
  if not ok or type(result) ~= "table" or result.exit_code ~= 0 then
    return nil
  end
  return decode_json(result.stdout)
end

M.fetch_issue = fetch_issue

-- deps = { github, repo, label, bot_login }
-- Returns an array of scope handles { number, repo, origin, state, text }.
-- Asserts a non-empty label FIRST (no silent default), then searches
-- `label:<label>` and shapes every OPEN result.
function M.list(deps)
  local search_label = label.require(deps and deps.label)
  local github = deps.github
  local repo = deps.repo
  local bot_login = deps.bot_login
  if type(github) ~= "table" or type(github.issue_search) ~= "function" then
    return {}
  end
  local ok, result = pcall(github.issue_search, repo, "label:" .. search_label, SEARCH_FIELDS, SEARCH_LIMIT)
  if not ok or type(result) ~= "table" or result.exit_code ~= 0 then
    return {}
  end
  local issues = decode_json(result.stdout)
  if type(issues) ~= "table" then
    return {}
  end
  local scopes = {}
  for _, issue in ipairs(issues) do
    if type(issue) == "table" and issue.number ~= nil and open_state(issue.state) then
      local full = fetch_issue(github, repo, issue.number) or issue
      table.insert(scopes, {
        number = issue.number,
        repo = repo,
        origin = "issue/" .. tostring(issue.number),
        state = issue.state,
        text = trusted_text(full, bot_login),
      })
    end
  end
  return scopes
end

return M
