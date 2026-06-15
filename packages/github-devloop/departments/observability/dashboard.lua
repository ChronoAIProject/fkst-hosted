local common = require("departments.observability.common")

local M = {}

function M.install_dashboard(core)
local dashboard_title = common.dashboard_title
local dashboard_label = common.dashboard_label
local dashboard_marker_prefix = common.dashboard_marker_prefix
local max_dashboard_body_len = common.max_dashboard_body_len
local max_dashboard_section_items = common.max_dashboard_section_items
local max_dashboard_title_len = common.max_dashboard_title_len

local function dashboard_deferred_if_deadline(deadline)
  return common.dashboard_deferred_if_deadline(core, deadline)
end

local function ensure_dashboard_label(repo, limits, deadline)
  local deferred = dashboard_deferred_if_deadline(deadline); if deferred ~= nil then return deferred end
  local existing = core.observability_exec(core.gh_dashboard_label_get_cmd(repo, dashboard_label), limits, deadline, "gh dashboard label get")
  if core.observability_result_deferred(existing) then return "deferred" end
  if existing.exit_code == 0 then
    return "exists"
  end
  if not common.command_indicates_not_found(existing) then
    error("github-devloop: gh dashboard label get failed: " .. tostring(existing.stderr))
  end

  deferred = dashboard_deferred_if_deadline(deadline); if deferred ~= nil then return deferred end
  local created = core.observability_exec(core.gh_dashboard_label_create_cmd(repo, dashboard_label), limits, deadline, "gh dashboard label create")
  if core.observability_result_deferred(created) then return "deferred" end
  if created.exit_code == 0 then
    log.info("github-devloop dept=observability tag=DASHBOARD_LABEL_CREATED label=" .. dashboard_label)
    return "created"
  end
  if common.command_indicates_already_exists(created) then
    return "exists"
  end
  error("github-devloop: gh dashboard label create failed: " .. tostring(created.stderr))
end

local function dashboard_input_path(repo, version, hash)
  local safe = core.sanitize_key(tostring(repo or "repo"), false):gsub("[/%s]+", "-")
  safe = safe:gsub("[^%w%._%-]", "-"):gsub("%-+", "-"):gsub("^%-+", ""):gsub("%-+$", "")
  if safe == "" then
    safe = "repo"
  end
  if #safe > 120 then
    safe = safe:sub(1, 120):gsub("%-+$", "")
  end
  local identity = core.sanitize_key(tostring(version or "unknown") .. "-" .. tostring(hash or "unknown"), false)
  identity = identity:gsub("[/%s]+", "-")
  identity = identity:gsub("[^%w%._%-]", "-"):gsub("%-+", "-"):gsub("^%-+", ""):gsub("%-+$", "")
  if identity == "" then
    identity = "unknown"
  end
  if #identity > 160 then
    identity = identity:sub(1, 160):gsub("%-+$", "")
  end
  return "/tmp/fkst-github-devloop-dashboard-" .. safe .. "-" .. identity .. ".json"
end

local function dashboard_marker(hash, generated_at)
  return dashboard_marker_prefix
    .. ' version="' .. tostring(generated_at or "")
    .. ' hash="' .. tostring(hash or "")
    .. '" generated_at="' .. tostring(generated_at or "")
    .. '" -->'
end

local function dashboard_marker_attr(body, name)
  local marker = tostring(body or ""):match("<!%-%- fkst:dashboard:v1[^>]*%-%->")
  if marker == nil then
    return nil
  end
  return marker:match(tostring(name) .. "=\"([^\"]+)\"")
end

local function dashboard_hash_from_body(body)
  return dashboard_marker_attr(body, "hash")
end

local function dashboard_version_from_body(body)
  return dashboard_marker_attr(body, "version") or dashboard_marker_attr(body, "generated_at")
end

local function dashboard_version_is_stale(target_version, current_version)
  if target_version == nil or current_version == nil then
    return false
  end
  local target = tostring(target_version)
  local current = tostring(current_version)
  if not target:match("^%d%d%d%d%-%d%d%-%d%dT%d%d:%d%d:%d%dZ$") then
    return false
  end
  if not current:match("^%d%d%d%d%-%d%d%-%d%dT%d%d:%d%d:%d%dZ$") then
    return false
  end
  return target <= current
end

local function split_included_headers(stdout)
  local text = tostring(stdout or "")
  local _head, body = text:match("^(.-)\r?\n\r?\n(.*)$")
  if body == nil then
    return "", text
  end
  return _head, body
end

local function parse_dashboard_issue_get(stdout)
  local _headers, body = split_included_headers(stdout)
  local decoded = json.decode(body or "{}")
  if type(decoded) ~= "table" then
    decoded = {}
  end
  local author_login = nil
  if type(decoded.author) == "table" and decoded.author.login ~= nil then
    author_login = tostring(decoded.author.login)
  elseif decoded.author_login ~= nil then
    author_login = tostring(decoded.author_login)
  elseif type(decoded.user) == "table" and decoded.user.login ~= nil then
    author_login = tostring(decoded.user.login)
  end
  return {
    number = tonumber(decoded.number),
    title = tostring(decoded.title or ""),
    author_login = author_login,
    body = tostring(decoded.body or ""),
    updated_at = decoded.updated_at or decoded.updatedAt,
  }
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
  title = core.neutralize_untrusted_comment_text(title)
  if title == "" then
    title = "(untitled)"
  end
  if #title > max_dashboard_title_len then
    title = core.truncate_utf8(title, max_dashboard_title_len - 3):gsub("%s+$", "") .. "..."
  end
  return title
end

local function entity_age_minutes(entity, now_seconds)
  if entity == nil or entity.state == nil then
    return nil
  end
  return core.stall_suspect_age_minutes(entity.state.version, now_seconds)
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

function core.render_observability_dashboard(args)
  local list = args and args.entities or {}
  local counts = args and args.counts or {}
  local stalls = args and args.stalls or {}
  local state_gap_report = args and args.state_gap_report or {}
  local now_seconds = args and args.now_seconds or now()
  local generated_at = os.date("!%Y-%m-%dT%H:%M:%SZ", now_seconds)
  local instance = core.read_env("FKST_GITHUB_BOT_LOGIN") or "unknown"
  local by_state = { unmanaged = {} }
  for _, state in ipairs(core._state_order) do
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
  for _, state in ipairs(core._state_order) do
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

  core.append_state_gap_dashboard_section(lines, state_gap_report)
  table.insert(lines, "")
  table.insert(lines, "## Footer")
  table.insert(lines, "- quota: not rendered")
  table.insert(lines, "- instance: " .. tostring(instance))
  table.insert(lines, "- generated-at: " .. generated_at)

  local stable = table.concat(lines, "\n")
  local hash = core._decimal_checksum(stable:gsub("%- generated%-at: [^\n]+", "- generated-at: <generated>"))
  local marker = dashboard_marker(hash, generated_at)
  local body = stable .. "\n\n" .. marker .. "\n"
  if #body > max_dashboard_body_len then
    local marker_suffix = "\n\n" .. marker .. "\n"
    body = core.truncate_utf8(body, max_dashboard_body_len - #marker_suffix) .. marker_suffix
  end
  return {
    body = body,
    hash = hash,
    version = generated_at,
    generated_at = generated_at,
  }
end

local function trusted_dashboard_issue(repo, bot_login, limits, deadline)
  local listed = core.observability_exec(core.gh_dashboard_issue_list_cmd(repo, dashboard_label), limits, deadline, "gh dashboard issue list")
  if core.observability_result_deferred(listed) then
    return "deferred"
  end
  if listed.exit_code ~= 0 then
    log.warn("github-devloop dept=observability tag=DASHBOARD_LOCATOR_FAILED"
      .. " locator=label-list"
      .. " label=" .. dashboard_label
      .. " auth_mode=" .. common.gh_auth_mode(core)
      .. " http_status=" .. common.stderr_http_status(listed.stderr)
      .. " exit_code=" .. tostring(listed.exit_code))
    error("github-devloop: gh dashboard issue list failed: " .. tostring(listed.stderr))
  end
  if tostring(listed.stdout or ""):match("^%s*$") then
    log.warn("github-devloop dept=observability tag=DASHBOARD_LOCATOR_FAILED locator=label-list label=" .. dashboard_label .. " reason=empty-output")
    error("github-devloop: gh dashboard issue list failed: empty output")
  end
  for _, issue in ipairs(core.parse_dashboard_issue_list(listed.stdout)) do
    if issue.author_login == bot_login
      and tostring(issue.body or ""):find(dashboard_marker_prefix, 1, true) ~= nil then
      return issue
    end
  end
  return nil
end

local function trusted_dashboard_issue_by_number(repo, issue_number, bot_login, limits, deadline)
  local view = core.observability_run_cmd(core.gh_dashboard_issue_get_cmd(repo, issue_number), limits, deadline, "gh dashboard issue get")
  if core.observability_result_deferred(view) then
    return "deferred"
  end
  local issue = parse_dashboard_issue_get(view.stdout)
  if issue.number == tonumber(issue_number)
    and issue.author_login == bot_login
    and tostring(issue.body or ""):find(dashboard_marker_prefix, 1, true) ~= nil then
    return issue
  end
  return nil
end

local function write_dashboard_input(repo, title, body)
  local path = dashboard_input_path(repo, dashboard_version_from_body(body), dashboard_hash_from_body(body))
  file.write(path, "{"
    .. '"title":' .. common.json_string(title)
    .. ',"body":' .. common.json_string(body)
    .. ',"labels":[' .. common.json_string(dashboard_label) .. "]"
    .. "}\n")
  return path
end

local function publish_observability_dashboard_locked(repo, dashboard, limits, deadline)
  if core.read_env("FKST_GITHUB_WRITE") ~= "1" then
    local deferred = dashboard_deferred_if_deadline(deadline)
    log.info("github-devloop dept=observability tag=DASHBOARD_DRY_RUN hash=" .. tostring(dashboard.hash))
    log.info(dashboard.body)
    return deferred or "dry-run"
  end

  local deferred = dashboard_deferred_if_deadline(deadline); if deferred ~= nil then return deferred end
  local bot_login = core.assert_trusted_bot_configured()
  deferred = ensure_dashboard_label(repo, limits, deadline); if deferred == "deferred" then return deferred end
  deferred = dashboard_deferred_if_deadline(deadline); if deferred ~= nil then return deferred end
  local current = trusted_dashboard_issue(repo, bot_login, limits, deadline)
  if current == "deferred" then return "deferred" end
  local current_version = current ~= nil and dashboard_version_from_body(current.body) or nil
  local current_hash = current ~= nil and dashboard_hash_from_body(current.body) or nil
  if current ~= nil and current_hash == dashboard.hash then
    log.info("github-devloop dept=observability tag=DASHBOARD_UNCHANGED issue=" .. tostring(current.number)
      .. " hash=" .. tostring(dashboard.hash))
    return "unchanged"
  end

  if current == nil then
    deferred = dashboard_deferred_if_deadline(deadline); if deferred ~= nil then return deferred end
    local path = write_dashboard_input(repo, dashboard_title, dashboard.body)
    local created = core.observability_run_cmd(core.gh_dashboard_issue_create_cmd(repo, path), limits, deadline, "gh dashboard issue create")
    if core.observability_result_deferred(created) then return "deferred" end
    log.info("github-devloop dept=observability tag=DASHBOARD_CREATED hash=" .. tostring(dashboard.hash))
    return "created"
  end

  if dashboard_version_is_stale(dashboard.version, current_version) then
    log.info("github-devloop dept=observability tag=DASHBOARD_STALE issue=" .. tostring(current.number)
      .. " current_version=" .. tostring(current_version or "")
      .. " target_version=" .. tostring(dashboard.version or "")
      .. " hash=" .. tostring(dashboard.hash))
    return "stale"
  end

  deferred = dashboard_deferred_if_deadline(deadline); if deferred ~= nil then return deferred end
  local refreshed = trusted_dashboard_issue_by_number(repo, current.number, bot_login, limits, deadline)
  if refreshed == "deferred" then return "deferred" end
  local refreshed_version = refreshed ~= nil and dashboard_version_from_body(refreshed.body) or nil
  local refreshed_hash = refreshed ~= nil and dashboard_hash_from_body(refreshed.body) or nil
  if refreshed ~= nil and tonumber(refreshed.number) == tonumber(current.number)
    and refreshed_hash == dashboard.hash then
    log.info("github-devloop dept=observability tag=DASHBOARD_UNCHANGED issue=" .. tostring(current.number)
      .. " hash=" .. tostring(dashboard.hash))
    return "unchanged"
  end
  if refreshed == nil or tonumber(refreshed.number) ~= tonumber(current.number)
    or refreshed_version ~= current_version then
    log.info("github-devloop dept=observability tag=DASHBOARD_CAS_MISMATCH issue=" .. tostring(current.number)
      .. " expected_version=" .. tostring(current_version or "")
      .. " actual_version=" .. tostring(refreshed_version or "")
      .. " hash=" .. tostring(dashboard.hash))
    return "cas-mismatch"
  end
  if dashboard_version_is_stale(dashboard.version, refreshed_version) then
    log.info("github-devloop dept=observability tag=DASHBOARD_STALE issue=" .. tostring(current.number)
      .. " current_version=" .. tostring(refreshed_version or "")
      .. " target_version=" .. tostring(dashboard.version or "")
      .. " hash=" .. tostring(dashboard.hash))
    return "stale"
  end
  local path = write_dashboard_input(repo, dashboard_title, dashboard.body)
  deferred = dashboard_deferred_if_deadline(deadline); if deferred ~= nil then return deferred end
  local updated = core.observability_exec(core.gh_dashboard_issue_update_cmd(repo, current.number, path), limits, deadline, "gh dashboard issue update")
  if core.observability_result_deferred(updated) then return "deferred" end
  if updated.exit_code ~= 0 then
    local stderr = tostring(updated.stderr or "")
    if stderr:find("412", 1, true) ~= nil or stderr:find("Precondition Failed", 1, true) ~= nil then
      log.info("github-devloop dept=observability tag=DASHBOARD_CAS_MISMATCH issue=" .. tostring(current.number)
        .. " expected_version=" .. tostring(refreshed_version or "")
        .. " actual_version=unknown"
        .. " reason=patch-precondition"
        .. " hash=" .. tostring(dashboard.hash))
      return "cas-mismatch"
    end
    error("github-devloop: gh dashboard issue update failed: " .. stderr)
  end
  log.info("github-devloop dept=observability tag=DASHBOARD_UPDATED issue=" .. tostring(current.number)
    .. " hash=" .. tostring(dashboard.hash))
  return "updated"
end

function core.publish_observability_dashboard(repo, dashboard, limits, deadline)
  if core.read_env("FKST_GITHUB_WRITE") ~= "1" then
    return publish_observability_dashboard_locked(repo, dashboard, limits, deadline)
  end
  local deferred = dashboard_deferred_if_deadline(deadline); if deferred ~= nil then return deferred end
  local outcome = nil
  with_lock("github-devloop/dashboard/" .. core.safe_repo(repo), function()
    outcome = publish_observability_dashboard_locked(repo, dashboard, limits, deadline)
  end)
  return outcome
end
end

return M
