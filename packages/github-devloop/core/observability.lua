local S = {}

function S.install(M)
local dept = "observability"
local stall_suspect_threshold_minutes = {
  thinking = 30,
  ready = 30,
  implementing = 90,
  ["pr-open"] = 30,
  reviewing = 60,
  fixing = 90,
  merging = 30,
}

local function run_cmd(cmd, timeout, error_class)
  local result = exec_sync({ cmd = cmd, timeout = timeout or 30 })
  if result.exit_code ~= 0 then
    error("github-devloop: " .. error_class .. " failed: " .. tostring(result.stderr))
  end
  return result
end

local function require_observe_repo()
  local repo = M.read_env("FKST_GITHUB_REPO")
  if repo == nil or M.safe_repo(repo) ~= tostring(repo) then
    error("github-devloop: FKST_GITHUB_REPO is required for observability")
  end
  return repo
end

local function require_observe_bot()
  local login = M.assert_trusted_bot_configured()
  if login == nil or tostring(login) == "" then
    error("github-devloop: FKST_GITHUB_BOT_LOGIN is required for observability")
  end
end

local function sorted_numbers(items)
  local numbers = {}
  local seen = {}
  for _, item in ipairs(items or {}) do
    local number = tonumber(item and item.number)
    local state = tostring(item and item.state or ""):lower()
    if number ~= nil and number >= 1 and number % 1 == 0 and state == "open" and not seen[number] then
      seen[number] = true
      table.insert(numbers, number)
    end
  end
  table.sort(numbers)
  return numbers
end

local function fetch_issue(repo, issue_number)
  local view = run_cmd(M.gh_issue_view_observe_cmd(repo, issue_number), 30, "gh observability issue view")
  return M.parse_issue_view_observe(view.stdout)
end

local function fetch_pr(repo, pr_number)
  local view = run_cmd(M.gh_pr_view_observe_cmd(repo, pr_number), 30, "gh observability PR view")
  return M.parse_pr_view_origin(view.stdout)
end

local function state_or_nil(state)
  if type(state) ~= "table" or state.state == nil then
    return nil
  end
  return state
end

local function put_issue_entity(entities, repo, issue_number, issue)
  local proposal_id = M.proposal_id(repo, issue_number)
  local issue_state = M.current_state(issue.comments, proposal_id)
  local link = M.pr_link_fact(issue.comments, proposal_id)
  local dependency_wait = M.dependency_wait_fact(issue.comments, proposal_id)
  local entity = entities[proposal_id] or {
    proposal_id = proposal_id,
    issue_number = tonumber(issue_number),
    pr_number = nil,
    state = nil,
    marker_source = nil,
    dependency_wait = nil,
  }
  entity.issue_number = tonumber(issue_number)
  if state_or_nil(issue_state) ~= nil then
    entity.state = issue_state
    entity.marker_source = "issue"
  end
  if link ~= nil then
    entity.pr_number = link.pr_number
  end
  entity.dependency_wait = dependency_wait
  entities[proposal_id] = entity
  return entity, link
end

local function put_pr_entity(entities, repo, pr_number, pr)
  local origin = M.pr_origin_fact(pr.comments)
  if origin == nil then
    return nil
  end
  local proposal_id = origin.proposal_id
  local pr_state = M.current_entity_state(pr.comments, proposal_id)
  local entity = entities[proposal_id] or {
    proposal_id = proposal_id,
    issue_number = origin.issue_number,
    pr_number = tonumber(pr_number),
    state = nil,
    marker_source = nil,
  }
  entity.issue_number = origin.issue_number
  entity.pr_number = tonumber(pr_number)
  if state_or_nil(pr_state) ~= nil then
    entity.state = pr_state
    entity.marker_source = "pr-comment"
  end
  entities[proposal_id] = entity
  return entity
end

local function observe_issue_candidates(repo, issue_numbers, entities, seen_prs)
  for _, issue_number in ipairs(issue_numbers) do
    local issue = fetch_issue(repo, issue_number)
    local entity, link = put_issue_entity(entities, repo, issue_number, issue)
    if link ~= nil and seen_prs[link.pr_number] == nil then
      seen_prs[link.pr_number] = true
      local pr = fetch_pr(repo, link.pr_number)
      put_pr_entity(entities, repo, link.pr_number, pr)
    elseif entity ~= nil and entity.pr_number ~= nil then
      seen_prs[entity.pr_number] = true
    end
  end
end

local function observe_pr_candidates(repo, pr_numbers, entities, seen_prs)
  for _, pr_number in ipairs(pr_numbers) do
    if seen_prs[pr_number] == nil then
      seen_prs[pr_number] = true
      local pr = fetch_pr(repo, pr_number)
      put_pr_entity(entities, repo, pr_number, pr)
    end
  end
end

local function entity_sort_key(entity)
  return tostring(entity.proposal_id or "")
end

local function log_entity(entity)
  local state = entity.state or {}
  log.info(M.observe_entity_log_line(entity.proposal_id, {
    state = state.state,
    version = state.version,
    marker_source = entity.marker_source,
    pr_number = entity.pr_number,
    marker_created_at = state.marker_created_at,
  }))
end

local function timestamp_epoch_seconds(timestamp)
  local year, month, day, hour, minute, second = tostring(timestamp or ""):match(
    "^(%d%d%d%d)%-(%d%d)%-(%d%d)T(%d%d)[%-:](%d%d)[%-:](%d%d)Z$"
  )
  if year == nil then
    return nil
  end
  year = tonumber(year)
  month = tonumber(month)
  day = tonumber(day)
  hour = tonumber(hour)
  minute = tonumber(minute)
  second = tonumber(second)
  if month < 1 or month > 12
    or day < 1 or day > 31
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

function M.stall_suspect_age_minutes(version, now_seconds)
  local marker_updated_at = M.version_updated_at(version)
  if marker_updated_at == "" then
    return nil
  end
  local marker_seconds = timestamp_epoch_seconds(marker_updated_at)
  local current_seconds = tonumber(now_seconds)
  if marker_seconds == nil or current_seconds == nil then
    return nil
  end
  local age_seconds = current_seconds - marker_seconds
  if age_seconds < 0 then
    return nil
  end
  return math.floor(age_seconds / 60)
end

function M.stall_suspect_threshold_minutes(state)
  return stall_suspect_threshold_minutes[state]
end

function M.stall_suspect_log_line(proposal_id, state, age_minutes, threshold_minutes)
  return table.concat({
    "github-devloop",
    "dept=" .. dept,
    "tag=STALL_SUSPECT",
    "proposal=" .. tostring(proposal_id or "unknown"),
    "state=" .. tostring(state or "unknown"),
    "age_minutes=" .. tostring(age_minutes or 0),
    "threshold_minutes=" .. tostring(threshold_minutes or 0),
  }, " ")
end

local function log_stall_suspect(entity, now_seconds)
  local state = entity.state and entity.state.state or nil
  local threshold = M.stall_suspect_threshold_minutes(state)
  if threshold == nil then
    return
  end
  if state == "ready" and entity.dependency_wait ~= nil then
    return
  end
  local age = M.stall_suspect_age_minutes(entity.state.version, now_seconds)
  if age == nil or age <= threshold then
    return
  end
  log.info(M.stall_suspect_log_line(entity.proposal_id, state, age, threshold))
end

local function log_summary(counts, total)
  local fields = {
    "github-devloop",
    "dept=" .. dept,
    "tag=OBSERVE_SUMMARY",
    "total=" .. tostring(total or 0),
  }
  for _, state in ipairs(M._state_order) do
    table.insert(fields, state .. "=" .. tostring(counts[state] or 0))
  end
  if counts.unmanaged ~= nil then
    table.insert(fields, "unmanaged=" .. tostring(counts.unmanaged))
  end
  log.info(table.concat(fields, " "))
end

function M.observe_entity_log_line(proposal_id, fields)
  return table.concat({
    "github-devloop",
    "dept=" .. dept,
    "tag=OBSERVE_ENTITY",
    "proposal_id=" .. tostring(proposal_id or "unknown"),
    "state=" .. tostring(fields and fields.state or "unmanaged"),
    "version=" .. tostring(fields and fields.version or ""),
    "marker_source=" .. tostring(fields and fields.marker_source or "none"),
    "pr=" .. tostring(fields and fields.pr_number or ""),
    "marker_created_at=" .. tostring(fields and fields.marker_created_at or ""),
  }, " ")
end

function M.observe_devloop_entities()
  require_observe_bot()
  local repo = require_observe_repo()
  local issue_candidates = {}
  local labels = { M._enabled_label }
  for _, state in ipairs(M._state_order) do
    table.insert(labels, M.state_label(state))
  end
  for _, label in ipairs(labels) do
    local issue_list = run_cmd(M.gh_issue_list_observe_cmd(repo, label), 60, "gh observability issue list")
    for _, issue in ipairs(M.parse_issue_list_observe(issue_list.stdout)) do
      table.insert(issue_candidates, issue)
    end
  end
  local pr_list = run_cmd(M.gh_pr_list_observe_cmd(repo), 60, "gh observability PR list")
  local issue_numbers = sorted_numbers(issue_candidates)
  local pr_numbers = sorted_numbers(M.parse_pr_list_observe(pr_list.stdout))
  local entities = {}
  local seen_prs = {}

  observe_issue_candidates(repo, issue_numbers, entities, seen_prs)
  observe_pr_candidates(repo, pr_numbers, entities, seen_prs)

  local list = {}
  for _, entity in pairs(entities) do
    table.insert(list, entity)
  end
  table.sort(list, function(a, b)
    return entity_sort_key(a) < entity_sort_key(b)
  end)

  local counts = {}
  local now_seconds = now()
  for _, entity in ipairs(list) do
    local state = entity.state and entity.state.state or "unmanaged"
    counts[state] = (counts[state] or 0) + 1
    log_entity(entity)
    log_stall_suspect(entity, now_seconds)
  end
  log_summary(counts, #list)

  return {
    entity_count = #list,
    counts = counts,
  }
end
end

return S
