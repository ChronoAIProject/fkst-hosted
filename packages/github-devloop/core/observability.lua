local S = {}

function S.install(M)
local dept = "observability"
local dashboard_title = "fkst-dev board"
local dashboard_marker_prefix = "<!-- fkst:dashboard:v1"
local max_dashboard_body_len = 12000
local max_dashboard_section_items = 40
local max_dashboard_title_len = 80
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

local function json_string(value)
  local text = tostring(value or "")
  text = text:gsub("\\", "\\\\")
  text = text:gsub('"', '\\"')
  text = text:gsub("\b", "\\b")
  text = text:gsub("\f", "\\f")
  text = text:gsub("\n", "\\n")
  text = text:gsub("\r", "\\r")
  text = text:gsub("\t", "\\t")
  text = text:gsub("[%z\1-\31]", function(char)
    return string.format("\\u%04x", char:byte())
  end)
  return '"' .. text .. '"'
end

local function dashboard_input_path(repo)
  local safe = M.sanitize_key(tostring(repo or "repo"), false):gsub("[/%s]+", "-")
  safe = safe:gsub("[^%w%._%-]", "-"):gsub("%-+", "-"):gsub("^%-+", ""):gsub("%-+$", "")
  if safe == "" then
    safe = "repo"
  end
  if #safe > 120 then
    safe = safe:sub(1, 120):gsub("%-+$", "")
  end
  return "/tmp/fkst-github-devloop-dashboard-" .. safe .. ".json"
end

local function dashboard_marker(hash, generated_at)
  return dashboard_marker_prefix
    .. ' hash="' .. tostring(hash or "")
    .. '" generated_at="' .. tostring(generated_at or "")
    .. '" -->'
end

local function dashboard_hash_from_body(body)
  return tostring(body or ""):match("<!%-%- fkst:dashboard:v1[^>]-hash=\"([^\"]+)\"[^>]*%-%->")
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
  entity.title = issue.title
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

local function entity_issue_ref(entity)
  if tonumber(entity.issue_number) ~= nil then
    return "#" .. tostring(entity.issue_number)
  end
  return tostring(entity.proposal_id or "unknown")
end

local function compact_title(value)
  local title = tostring(value or ""):gsub("%c", " "):gsub("%s+", " ")
  title = title:gsub("^%s+", ""):gsub("%s+$", "")
  title = M.neutralize_untrusted_comment_text(title)
  if title == "" then
    title = "(untitled)"
  end
  if #title > max_dashboard_title_len then
    title = M._utf8_safe_truncate(title, max_dashboard_title_len - 3):gsub("%s+$", "") .. "..."
  end
  return title
end

local function entity_age_minutes(entity, now_seconds)
  if entity == nil or entity.state == nil then
    return nil
  end
  return M.stall_suspect_age_minutes(entity.state.version, now_seconds)
end

local function format_age(age_minutes)
  if tonumber(age_minutes) == nil then
    return "age unknown"
  end
  local minutes = tonumber(age_minutes)
  if minutes < 60 then
    return tostring(minutes) .. "m"
  end
  local hours = math.floor(minutes / 60)
  local rest = minutes % 60
  if hours < 48 then
    return tostring(hours) .. "h " .. tostring(rest) .. "m"
  end
  local days = math.floor(hours / 24)
  local day_hours = hours % 24
  return tostring(days) .. "d " .. tostring(day_hours) .. "h"
end

local function entity_line(entity, now_seconds)
  local state = entity.state and entity.state.state or "unmanaged"
  local parts = {
    "- " .. entity_issue_ref(entity),
    compact_title(entity.title),
    "-",
    tostring(state) .. ",",
    format_age(entity_age_minutes(entity, now_seconds)),
  }
  if tonumber(entity.pr_number) ~= nil then
    table.insert(parts, "(PR #" .. tostring(entity.pr_number) .. ")")
  end
  if entity.dependency_wait ~= nil then
    table.insert(parts, "[dependency-wait]")
  end
  return table.concat(parts, " ")
end

local function append_entity_lines(lines, entities, now_seconds)
  if #entities == 0 then
    table.insert(lines, "- None")
    return
  end
  local shown = 0
  for _, entity in ipairs(entities) do
    if shown >= max_dashboard_section_items then
      table.insert(lines, "- ... " .. tostring(#entities - shown) .. " more")
      return
    end
    table.insert(lines, entity_line(entity, now_seconds))
    shown = shown + 1
  end
end

local function append_state_section(lines, title, state, by_state, now_seconds)
  table.insert(lines, "")
  table.insert(lines, "## " .. title)
  append_entity_lines(lines, by_state[state] or {}, now_seconds)
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

function M.stall_suspect_age_minutes(version, now_seconds)
  local marker_updated_at = M.version_updated_at(version)
  if marker_updated_at == "" then
    return nil
  end
  local marker_seconds = M.iso_timestamp_epoch_seconds(marker_updated_at)
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
  return {
    entity = entity,
    state = state,
    age_minutes = age,
    threshold_minutes = threshold,
  }
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

function M.render_observability_dashboard(args)
  local list = args and args.entities or {}
  local counts = args and args.counts or {}
  local stalls = args and args.stalls or {}
  local now_seconds = args and args.now_seconds or now()
  local generated_at = os.date("!%Y-%m-%dT%H:%M:%SZ", now_seconds)
  local instance = M.read_env("FKST_GITHUB_BOT_LOGIN") or "unknown"
  local by_state = { unmanaged = {} }
  for _, state in ipairs(M._state_order) do
    by_state[state] = {}
  end
  for _, entity in ipairs(list) do
    local state = entity.state and entity.state.state or "unmanaged"
    by_state[state] = by_state[state] or {}
    table.insert(by_state[state], entity)
  end

  local lines = {
    "# " .. dashboard_title,
    "",
    "Live read-only dashboard generated from trusted fkst-dev markers. Chinese: &#27492;&#30475;&#26495;&#21482;&#26159;&#21487;&#20449; marker &#30340;&#21482;&#35835;&#27966;&#29983;&#35270;&#22270;&#65292;&#19981;&#26159;&#20107;&#23454;&#28304;&#12290;",
    "",
    "## Now working",
  }
  local working = {}
  for _, state in ipairs({ "implementing", "pr-open", "reviewing", "fixing", "merge-ready", "merging" }) do
    for _, entity in ipairs(by_state[state] or {}) do
      table.insert(working, entity)
    end
  end
  append_entity_lines(lines, working, now_seconds)

  table.insert(lines, "")
  table.insert(lines, "## Board by state")
  table.insert(lines, "Total: " .. tostring(#list))
  for _, state in ipairs(M._state_order) do
    table.insert(lines, "- " .. tostring(state) .. ": " .. tostring(counts[state] or 0))
  end
  if counts.unmanaged ~= nil then
    table.insert(lines, "- unmanaged: " .. tostring(counts.unmanaged))
  end

  append_state_section(lines, "Ready", "ready", by_state, now_seconds)
  append_state_section(lines, "Blocked", "blocked", by_state, now_seconds)
  append_state_section(lines, "Review meta", "review-meta", by_state, now_seconds)
  append_state_section(lines, "Thinking", "thinking", by_state, now_seconds)

  table.insert(lines, "")
  table.insert(lines, "## Stall suspects")
  if #stalls == 0 then
    table.insert(lines, "- None")
  else
    local shown = 0
    for _, stall in ipairs(stalls) do
      if shown >= max_dashboard_section_items then
        table.insert(lines, "- ... " .. tostring(#stalls - shown) .. " more")
        break
      end
      table.insert(lines, entity_line(stall.entity, now_seconds)
        .. " (threshold " .. tostring(stall.threshold_minutes) .. "m)")
      shown = shown + 1
    end
  end

  table.insert(lines, "")
  table.insert(lines, "## Recent transitions")
  table.insert(lines, "- Not rendered: no existing low-cost transition history source is available to this department.")
  table.insert(lines, "")
  table.insert(lines, "## Footer")
  table.insert(lines, "- quota: not rendered")
  table.insert(lines, "- instance: " .. tostring(instance))
  table.insert(lines, "- generated-at: " .. generated_at)

  local stable = table.concat(lines, "\n")
  local hash = M._decimal_checksum(stable:gsub("%- generated%-at: [^\n]+", "- generated-at: <generated>"))
  local marker = dashboard_marker(hash, generated_at)
  local body = stable .. "\n\n" .. marker .. "\n"
  if #body > max_dashboard_body_len then
    local marker_suffix = "\n\n" .. marker .. "\n"
    body = M._utf8_safe_truncate(body, max_dashboard_body_len - #marker_suffix) .. marker_suffix
  end
  return {
    body = body,
    hash = hash,
    generated_at = generated_at,
  }
end

local function trusted_dashboard_issue(repo, bot_login)
  local search = run_cmd(M.gh_dashboard_issue_search_cmd(repo), 30, "gh dashboard issue search")
  for _, issue in ipairs(M.parse_dashboard_issue_search(search.stdout)) do
    if issue.author_login == bot_login
      and tostring(issue.body or ""):find(dashboard_marker_prefix, 1, true) ~= nil then
      return issue
    end
  end
  return nil
end

local function write_dashboard_input(repo, title, body)
  local path = dashboard_input_path(repo)
  file.write(path, "{"
    .. '"title":' .. json_string(title)
    .. ',"body":' .. json_string(body)
    .. "}\n")
  return path
end

function M.publish_observability_dashboard(repo, dashboard)
  if M.read_env("FKST_GITHUB_WRITE") ~= "1" then
    log.info("github-devloop dept=observability tag=DASHBOARD_DRY_RUN hash=" .. tostring(dashboard.hash))
    log.info(dashboard.body)
    return "dry-run"
  end

  local bot_login = M.assert_trusted_bot_configured()
  local current = trusted_dashboard_issue(repo, bot_login)
  if current ~= nil and dashboard_hash_from_body(current.body) == dashboard.hash then
    log.info("github-devloop dept=observability tag=DASHBOARD_UNCHANGED issue=" .. tostring(current.number)
      .. " hash=" .. tostring(dashboard.hash))
    return "unchanged"
  end

  local path = write_dashboard_input(repo, dashboard_title, dashboard.body)
  if current == nil then
    run_cmd(M.gh_dashboard_issue_create_cmd(repo, path), 30, "gh dashboard issue create")
    log.info("github-devloop dept=observability tag=DASHBOARD_CREATED hash=" .. tostring(dashboard.hash))
    return "created"
  end
  run_cmd(M.gh_dashboard_issue_update_cmd(repo, current.number, path), 30, "gh dashboard issue update")
  log.info("github-devloop dept=observability tag=DASHBOARD_UPDATED issue=" .. tostring(current.number)
    .. " hash=" .. tostring(dashboard.hash))
  return "updated"
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
  local stalls = {}
  for _, entity in ipairs(list) do
    local state = entity.state and entity.state.state or "unmanaged"
    counts[state] = (counts[state] or 0) + 1
    log_entity(entity)
    local stall = log_stall_suspect(entity, now_seconds)
    if stall ~= nil then
      table.insert(stalls, stall)
    end
  end
  log_summary(counts, #list)
  local dashboard = M.render_observability_dashboard({
    entities = list,
    counts = counts,
    stalls = stalls,
    now_seconds = now_seconds,
  })
  M.publish_observability_dashboard(repo, dashboard)

  return {
    entity_count = #list,
    counts = counts,
    dashboard_hash = dashboard.hash,
  }
end
end

return S
