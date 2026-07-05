local base_ids = require("devloop.base_ids")
local devloop_base = require("devloop.base")
local devloop_entity = require("devloop.entity")
local devloop_logging = require("devloop.logging")
local marker = require("core.marker")
local parsers_issue = require("devloop.parsers.issue")
local parsers_misc = require("devloop.parsers.misc")
local commands = require("devloop.commands")

local M = {}

M.MAX_ORIGINS_PER_TICK = 25
M.ISSUE_LIST_TIMEOUT_SECONDS = 30
M.ISSUE_VIEW_TIMEOUT_SECONDS = 30
M.MATERIALIZATION_CURSOR_PREFIX = "github-devloop-workflow/materialization/cursor/"

local function tick_proposal_id()
  return "github-devloop-workflow/materialization"
end

local function sort_by_number(items)
  table.sort(items, function(left, right)
    return tonumber(left.number or 0) < tonumber(right.number or 0)
  end)
  return items
end

function M.safe_source_ref(repo, issue_number)
  return devloop_entity.issue_source_ref(repo, issue_number)
end

function M.log_decision(dept, proposal_id, from_state, to_state, outcome, reason)
  devloop_logging.log_cas_decision(
    dept,
    proposal_id or tick_proposal_id(),
    { state = nil, version = nil },
    from_state,
    to_state,
    outcome,
    reason
  )
end

function M.read_repo(deps)
  if type(deps) == "table" and type(deps.read_repo) == "function" then
    return deps.read_repo()
  end
  local repo = devloop_base.read_env("FKST_GITHUB_REPO")
  if repo == nil or not base_ids.issue_ref_round_trips(repo, 1) then
    return nil
  end
  return repo
end

function M.bounded_slice(core, dept, repo, issues)
  local all = sort_by_number(issues or {})
  local total = #all
  if total <= M.MAX_ORIGINS_PER_TICK then
    cache_set(M.MATERIALIZATION_CURSOR_PREFIX .. base_ids.safe_repo(repo), "0")
    return all, 0
  end

  local cursor_key = M.MATERIALIZATION_CURSOR_PREFIX .. base_ids.safe_repo(repo)
  local cursor = tonumber(cache_get(cursor_key) or "0") or 0
  if cursor < 0 or cursor >= total then
    cursor = 0
  end

  local selected = {}
  for offset = 0, M.MAX_ORIGINS_PER_TICK - 1 do
    local index = ((cursor + offset) % total) + 1
    selected[#selected + 1] = all[index]
  end
  cache_set(cursor_key, tostring((cursor + #selected) % total))

  local deferred = total - #selected
  M.log_decision(
    dept,
    tick_proposal_id(),
    "tick",
    "discover",
    "deferred-cap",
    tostring(deferred) .. " open issues deferred by MAX_ORIGINS_PER_TICK=" .. tostring(M.MAX_ORIGINS_PER_TICK)
  )
  return selected, deferred
end

function M.list_open_issues(core, deps, repo)
  if type(deps.list_open_issues) == "function" then
    return deps.list_open_issues(core, repo)
  end
  local result = commands.gh_issue_list_observe(repo, nil, nil, nil, M.ISSUE_LIST_TIMEOUT_SECONDS)
  if type(result) ~= "table" or result.exit_code ~= 0 then
    error("github-devloop-workflow: materialization-issue-list-failed: materialization issue list failed: " .. tostring(result and result.stderr or "nil result"))
  end
  return parsers_issue.parse_issue_list_observe(result.stdout)
end

function M.read_issue(core, deps, repo, issue_number)
  if type(deps.read_issue) == "function" then
    return deps.read_issue(core, repo, issue_number)
  end
  local result = commands.gh_issue_view(
    repo,
    issue_number,
    "title,body,updatedAt,labels,comments,state,assignees,author",
    M.ISSUE_VIEW_TIMEOUT_SECONDS
  )
  if type(result) ~= "table" or result.exit_code ~= 0 then
    error("github-devloop-workflow: materialization-issue-view-failed: materialization issue view failed: " .. tostring(result and result.stderr or "nil result"))
  end
  local current = parsers_issue.parse_issue_view_intake_judge(core, result.stdout)
  current.repo = repo
  current.number = issue_number
  return current
end

function M.trusted_comments(core, comments)
  return parsers_misc._trusted_marker_comments(comments or {})
end

function M.latest_blueprint(core, current, origin)
  local fact = nil
  for _, comment in ipairs(M.trusted_comments(core, current and current.comments)) do
    fact = marker.parse_blueprint_marker(parsers_misc.comment_body(comment), origin) or fact
  end
  return fact
end

function M.latest_terminal(core, current, origin)
  local fact = nil
  for _, comment in ipairs(M.trusted_comments(core, current and current.comments)) do
    fact = marker.parse_terminal_marker(parsers_misc.comment_body(comment), origin) or fact
  end
  return fact
end

function M.materialization_facts(core, current, origin)
  local facts = {}
  for _, comment in ipairs(M.trusted_comments(core, current and current.comments)) do
    for _, fact in ipairs(marker.parse_materialization_markers(parsers_misc.comment_body(comment), origin)) do
      facts[#facts + 1] = fact
    end
  end
  return facts
end

return M
