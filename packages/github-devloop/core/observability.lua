local S = {}

function S.install(M)
local dept = "observability"
local dashboard_title = "fkst-dev board"
local dashboard_label = "fkst-dashboard"
local dashboard_marker_prefix = "<!-- fkst:dashboard:v1"
local max_dashboard_body_len = 12000
local max_dashboard_section_items = 40
local max_dashboard_title_len = 80
local max_reap_reason_len = 180
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
  local result = M.gh_exec({ cmd = cmd, timeout = timeout or 30 })
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

local function stderr_http_status(stderr)
  local text = tostring(stderr or "")
  local status = text:match("[Hh][Tt][Tt][Pp][^%d]*(%d%d%d)")
    or text:match("status[^\n%d]*(%d%d%d)")
  return status or "unknown"
end

local function gh_auth_mode()
  if M.env_present("GH_TOKEN") or M.env_present("GITHUB_TOKEN") then
    return "env-token"
  end
  return "gh-auth"
end

local function command_indicates_not_found(result)
  local stderr = tostring(result and result.stderr or "")
  return stderr:find("404", 1, true) ~= nil
    or stderr:lower():find("not found", 1, true) ~= nil
end

local function command_indicates_already_exists(result)
  local stderr = tostring(result and result.stderr or ""):lower()
  return stderr:find("already exists", 1, true) ~= nil
    or stderr:find("name already exists", 1, true) ~= nil
    or stderr:find("422", 1, true) ~= nil
    or stderr:find("409", 1, true) ~= nil
end

local function ensure_dashboard_label(repo, limits, deadline)
  local existing = M.observability_exec(M.gh_dashboard_label_get_cmd(repo, dashboard_label), limits, deadline, "gh dashboard label get")
  if existing.exit_code == 0 then
    return "exists"
  end
  if not command_indicates_not_found(existing) then
    error("github-devloop: gh dashboard label get failed: " .. tostring(existing.stderr))
  end

  local created = M.observability_exec(M.gh_dashboard_label_create_cmd(repo, dashboard_label), limits, deadline, "gh dashboard label create")
  if created.exit_code == 0 then
    log.info("github-devloop dept=observability tag=DASHBOARD_LABEL_CREATED label=" .. dashboard_label)
    return "created"
  end
  if command_indicates_already_exists(created) then
    return "exists"
  end
  error("github-devloop: gh dashboard label create failed: " .. tostring(created.stderr))
end

local function dashboard_input_path(repo, version, hash)
  local safe = M.sanitize_key(tostring(repo or "repo"), false):gsub("[/%s]+", "-")
  safe = safe:gsub("[^%w%._%-]", "-"):gsub("%-+", "-"):gsub("^%-+", ""):gsub("%-+$", "")
  if safe == "" then
    safe = "repo"
  end
  if #safe > 120 then
    safe = safe:sub(1, 120):gsub("%-+$", "")
  end
  local identity = M.sanitize_key(tostring(version or "unknown") .. "-" .. tostring(hash or "unknown"), false)
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
  local head, body = text:match("^(.-)\r?\n\r?\n(.*)$")
  if head == nil then
    return "", text
  end
  return head, body
end

local function header_value(headers, name)
  local target = tostring(name or ""):lower()
  for line in tostring(headers or ""):gmatch("[^\r\n]+") do
    local key, value = line:match("^%s*([^:]+):%s*(.-)%s*$")
    if key ~= nil and key:lower() == target then
      return value
    end
  end
  return nil
end

local function parse_dashboard_issue_get(stdout)
  local headers, body = split_included_headers(stdout)
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
    etag = header_value(headers, "etag"),
  }
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

local function fetch_issue(repo, issue_number, limits, deadline)
  local view = M.observability_run_cmd(M.gh_issue_view_observe_cmd(repo, issue_number), limits, deadline, "gh observability issue view")
  return M.parse_issue_view_observe(view.stdout)
end

local function fetch_pr(repo, pr_number, limits, deadline)
  local view = M.observability_run_cmd(M.gh_pr_view_observe_cmd(repo, pr_number), limits, deadline, "gh observability PR view")
  return M.parse_pr_view_origin(view.stdout)
end

local function reaper_body_path(repo, pr_number, proposal_id)
  local safe_repo = M.safe_repo(repo):gsub("[/%s]+", "-")
  local safe_issue = M.sanitize_key(tostring(proposal_id or "unknown"), false):gsub("[/%s]+", "-")
  local identity = safe_repo .. "-pr-" .. tostring(pr_number) .. "-" .. safe_issue
  identity = identity:gsub("[^%w%._%-]", "-"):gsub("%-+", "-"):gsub("^%-+", ""):gsub("%-+$", "")
  if identity == "" then
    identity = "orphan-pr"
  end
  if #identity > 180 then
    identity = identity:sub(1, 180):gsub("%-+$", "")
  end
  return "/tmp/fkst-github-devloop-reap-" .. identity .. ".md"
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
  entity.parent_issue = issue
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
  entity.pr_origin = origin
  entity.pr = pr
  if state_or_nil(pr_state) ~= nil then
    entity.state = pr_state
    entity.marker_source = "pr-comment"
  end
  entities[proposal_id] = entity
  return entity
end

local function orphan_reap_log_line(repo, pr_number, proposal_id, action, reason)
  return table.concat({
    "github-devloop",
    "dept=" .. dept,
    "tag=REAP",
    "repo=" .. tostring(repo or ""),
    "pr=" .. tostring(pr_number or ""),
    "proposal_id=" .. tostring(proposal_id or "unknown"),
    "action=" .. tostring(action or "skip"),
    "reason=" .. tostring(reason or ""),
  }, " ")
end

local function successor_issue_numbers(comments, proposal_id)
  local successors = {}
  local seen = {}
  local dedup_prefix = "decompose/" .. tostring(proposal_id) .. "/"
  local marker_pattern = "<!%-%- fkst:github%-proxy:issue%-created:v1.-%-%->"
  for _, comment in ipairs(M._trusted_marker_comments(comments or {})) do
    for marker in M._comment_body(comment):gmatch(marker_pattern) do
      local dedup = marker:match('dedup="([^"]+)"')
      local issue = marker:match('issue="([^"]+)"')
      if tostring(dedup or ""):sub(1, #dedup_prefix) == dedup_prefix
        and M._is_positive_pr_number(issue)
        and not seen[tostring(issue)] then
        seen[tostring(issue)] = true
        table.insert(successors, tonumber(issue))
      end
    end
  end
  table.sort(successors)
  return successors
end

local function successor_summary(successors, fallback_count)
  if #successors == 0 then
    return tostring(fallback_count or 0) .. " successor issue(s)"
  end
  local refs = {}
  for _, issue in ipairs(successors) do
    table.insert(refs, "#" .. tostring(issue))
  end
  return table.concat(refs, ", ")
end

local function terminal_parent_reason(parent_issue, entity)
  local proposal_id = entity.proposal_id
  local pr_comments = entity.pr and entity.pr.comments or {}
  local successors = successor_issue_numbers(pr_comments, proposal_id)
  if tostring(parent_issue and parent_issue.state or ""):upper() == "CLOSED" then
    return {
      code = "parent-closed",
      text = "Parent issue #" .. tostring(select(2, M.parse_proposal_id(proposal_id)) or "unknown") .. " is closed.",
      successors = successors,
    }
  end
  local decomposed = M.decomposed_fact(pr_comments, proposal_id)
    or M.decomposed_fact(parent_issue and parent_issue.comments or {}, proposal_id)
  if decomposed ~= nil
    and tostring(decomposed.pr_number or "") == tostring(entity.pr_number or "")
    and #successors >= decomposed.count then
    return {
      code = "parent-decomposed",
      text = "Parent issue #"
        .. tostring(select(2, M.parse_proposal_id(proposal_id)) or "unknown")
        .. " has a trusted decomposed marker with successors: "
        .. successor_summary(successors, decomposed.count),
      successors = successors,
    }
  end
  return nil
end

local function reaper_comment_body(proposal_id, pr_number, reason)
  local repo, issue_number = M.parse_proposal_id(proposal_id)
  local parent_ref = "#" .. tostring(issue_number or "unknown")
  local reason_text = tostring(reason and reason.text or "")
  if #reason_text > max_reap_reason_len then
    reason_text = M.truncate_utf8(reason_text, max_reap_reason_len)
  end
  return "github-devloop reaped this managed PR because its parent issue is terminal.\n\n"
    .. "Parent: " .. parent_ref .. "\n"
    .. "Reason: " .. reason_text .. "\n"
    .. "Successors: " .. successor_summary(reason and reason.successors or {}, nil) .. "\n"
    .. "Branch cleanup is intentionally left to a separate manual or managed path.\n\n"
    .. M.orphan_reaped_marker(proposal_id, pr_number, reason and reason.code or "parent-terminal")
    .. "\n"
end

local function reap_orphan_pr(repo, entity)
  if entity == nil or entity.pr_origin == nil or entity.pr == nil then
    return
  end
  local origin = entity.pr_origin
  local proposal_id = origin.proposal_id
  local pr_number = entity.pr_number
  if not M.is_devloop_issue_branch(origin.branch) then
    log.info(orphan_reap_log_line(repo, pr_number, proposal_id, "skip", "non-devloop-branch"))
    return
  end
  if tostring(entity.pr.head_ref_name or "") ~= tostring(origin.branch or "") then
    log.info(orphan_reap_log_line(repo, pr_number, proposal_id, "skip", "branch-mismatch"))
    return
  end
  if tostring(entity.pr.state or ""):upper() ~= "OPEN" then
    return
  end
  if M.has_orphan_reaped_marker(entity.pr.comments, proposal_id, pr_number) then
    log.info(orphan_reap_log_line(repo, pr_number, proposal_id, "skip-idempotent", "orphan-reaped-marker-visible"))
    return
  end

  local parent = entity.parent_issue or fetch_issue(repo, origin.issue_number, entity.observability_limits, entity.observability_deadline)
  local parent_state = M.current_state(parent.comments, proposal_id)
  local reason = terminal_parent_reason(parent, entity)
  if reason == nil then
    log.info(orphan_reap_log_line(repo, pr_number, proposal_id, "skip", "parent-active"))
    return
  end
  if reason.code ~= "parent-decomposed"
    and tostring(parent.state or ""):upper() ~= "CLOSED"
    and parent_state.state ~= "blocked"
    and parent_state.state ~= "impl-failed"
    and parent_state.state ~= "merged" then
    log.info(orphan_reap_log_line(repo, pr_number, proposal_id, "skip", "parent-marker-not-terminal"))
    return
  end

  if M.write_mode() ~= "real" then
    log.info(orphan_reap_log_line(repo, pr_number, proposal_id, "dry-run", reason.code))
    return
  end

  M.observability_run_cmd(M.gh_pr_close_cmd(repo, pr_number), entity.observability_limits, entity.observability_deadline, "gh orphan PR close")
  M.invalidate_entity_after_write(repo, "pr", pr_number)
  local path = reaper_body_path(repo, pr_number, proposal_id)
  file.write(path, reaper_comment_body(proposal_id, pr_number, reason))
  M.observability_run_cmd(M.gh_pr_comment_cmd(repo, pr_number, path), entity.observability_limits, entity.observability_deadline, "gh orphan PR reaper comment")
  M.invalidate_entity_after_write(repo, "pr", pr_number)
  log.info(orphan_reap_log_line(repo, pr_number, proposal_id, "closed", reason.code))
end

local function reap_orphan_prs(repo, entities)
  for _, entity in ipairs(entities or {}) do
    reap_orphan_pr(repo, entity)
  end
end

local function observe_issue_candidate(repo, issue_number, entities, seen_prs, limits, deadline, budget)
  local issue_views = 0
  local pr_views = 0
  if (budget.remaining or 0) <= 0 or not M.observability_has_budget(deadline) then
    return issue_views, pr_views
  end
  local issue = fetch_issue(repo, issue_number, limits, deadline)
  budget.remaining = budget.remaining - 1
  issue_views = issue_views + 1
  local entity, link = put_issue_entity(entities, repo, issue_number, issue)
  if link ~= nil and seen_prs[link.pr_number] == nil then
    if (budget.remaining or 0) <= 0 or not M.observability_has_budget(deadline) then
      return issue_views, pr_views
    end
    seen_prs[link.pr_number] = true
    local pr = fetch_pr(repo, link.pr_number, limits, deadline)
    budget.remaining = budget.remaining - 1
    pr_views = pr_views + 1
    put_pr_entity(entities, repo, link.pr_number, pr)
  elseif entity ~= nil and entity.pr_number ~= nil then
    seen_prs[entity.pr_number] = true
  end
  return issue_views, pr_views
end

local function observe_pr_candidate(repo, pr_number, entities, seen_prs, limits, deadline, budget)
  local pr_views = 0
  if (budget.remaining or 0) <= 0 or not M.observability_has_budget(deadline) then
    return pr_views
  end
  if seen_prs[pr_number] == nil then
    seen_prs[pr_number] = true
    local pr = fetch_pr(repo, pr_number, limits, deadline)
    budget.remaining = budget.remaining - 1
    pr_views = pr_views + 1
    put_pr_entity(entities, repo, pr_number, pr)
  end
  return pr_views
end

local function observe_candidates(repo, candidates, entities, seen_prs, limits, deadline)
  local budget = { remaining = limits.entity_cap }
  local processed_issues = 0
  local processed_prs = 0
  for _, candidate in ipairs(candidates or {}) do
    if budget.remaining <= 0 or not M.observability_has_budget(deadline) then
      break
    end
    if candidate.kind == "issue" then
      local issue_views, pr_views = observe_issue_candidate(repo, candidate.number, entities, seen_prs, limits, deadline, budget)
      processed_issues = processed_issues + issue_views
      processed_prs = processed_prs + pr_views
    elseif candidate.kind == "pr" then
      processed_prs = processed_prs + observe_pr_candidate(repo, candidate.number, entities, seen_prs, limits, deadline, budget)
    end
  end
  return processed_issues, processed_prs, budget.remaining
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
    title = M.truncate_utf8(title, max_dashboard_title_len - 3):gsub("%s+$", "") .. "..."
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
  local state_gap_report = args and args.state_gap_report or {}
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

  M.append_state_gap_dashboard_section(lines, state_gap_report)
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
    body = M.truncate_utf8(body, max_dashboard_body_len - #marker_suffix) .. marker_suffix
  end
  return {
    body = body,
    hash = hash,
    version = generated_at,
    generated_at = generated_at,
  }
end

local function trusted_dashboard_issue(repo, bot_login, limits, deadline)
  local listed = M.observability_exec(M.gh_dashboard_issue_list_cmd(repo, dashboard_label), limits, deadline, "gh dashboard issue list")
  if listed.exit_code ~= 0 then
    log.warn("github-devloop dept=observability tag=DASHBOARD_LOCATOR_FAILED"
      .. " locator=label-list"
      .. " label=" .. dashboard_label
      .. " auth_mode=" .. gh_auth_mode()
      .. " http_status=" .. stderr_http_status(listed.stderr)
      .. " exit_code=" .. tostring(listed.exit_code))
    error("github-devloop: gh dashboard issue list failed: " .. tostring(listed.stderr))
  end
  for _, issue in ipairs(M.parse_dashboard_issue_list(listed.stdout)) do
    if issue.author_login == bot_login
      and tostring(issue.body or ""):find(dashboard_marker_prefix, 1, true) ~= nil then
      return issue
    end
  end
  return nil
end

local function trusted_dashboard_issue_by_number(repo, issue_number, bot_login, limits, deadline)
  local view = M.observability_run_cmd(M.gh_dashboard_issue_get_cmd(repo, issue_number), limits, deadline, "gh dashboard issue get")
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
    .. '"title":' .. json_string(title)
    .. ',"body":' .. json_string(body)
    .. ',"labels":[' .. json_string(dashboard_label) .. "]"
    .. "}\n")
  return path
end

function M.publish_observability_dashboard(repo, dashboard, limits, deadline)
  if M.read_env("FKST_GITHUB_WRITE") ~= "1" then
    log.info("github-devloop dept=observability tag=DASHBOARD_DRY_RUN hash=" .. tostring(dashboard.hash))
    log.info(dashboard.body)
    return "dry-run"
  end

  local bot_login = M.assert_trusted_bot_configured()
  ensure_dashboard_label(repo, limits, deadline)
  local current = trusted_dashboard_issue(repo, bot_login, limits, deadline)
  local current_version = current ~= nil and dashboard_version_from_body(current.body) or nil
  local current_hash = current ~= nil and dashboard_hash_from_body(current.body) or nil
  if current ~= nil and current_hash == dashboard.hash then
    log.info("github-devloop dept=observability tag=DASHBOARD_UNCHANGED issue=" .. tostring(current.number)
      .. " hash=" .. tostring(dashboard.hash))
    return "unchanged"
  end

  if current == nil then
    local path = write_dashboard_input(repo, dashboard_title, dashboard.body)
    M.observability_run_cmd(M.gh_dashboard_issue_create_cmd(repo, path), limits, deadline, "gh dashboard issue create")
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

  local refreshed = trusted_dashboard_issue_by_number(repo, current.number, bot_login, limits, deadline)
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
  if refreshed.etag == nil or tostring(refreshed.etag) == "" then
    log.info("github-devloop dept=observability tag=DASHBOARD_CAS_MISMATCH issue=" .. tostring(current.number)
      .. " expected_version=" .. tostring(current_version or "")
      .. " actual_version=" .. tostring(refreshed_version or "")
      .. " reason=missing-etag"
      .. " hash=" .. tostring(dashboard.hash))
    return "cas-mismatch"
  end

  local path = write_dashboard_input(repo, dashboard_title, dashboard.body)
  local updated = M.observability_exec(M.gh_dashboard_issue_update_cmd(repo, current.number, path, refreshed.etag), limits, deadline, "gh dashboard issue update")
  if updated.exit_code ~= 0 then
    local stderr = tostring(updated.stderr or "")
    if stderr:find("412", 1, true) ~= nil or stderr:find("Precondition Failed", 1, true) ~= nil then
      log.info("github-devloop dept=observability tag=DASHBOARD_CAS_MISMATCH issue=" .. tostring(current.number)
        .. " expected_version=" .. tostring(refreshed_version or "")
        .. " actual_version=unknown"
        .. " reason=etag-precondition"
        .. " hash=" .. tostring(dashboard.hash))
      return "cas-mismatch"
    end
    error("github-devloop: gh dashboard issue update failed: " .. stderr)
  end
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

function M.observe_devloop_entities(event)
  require_observe_bot()
  local repo = require_observe_repo()
  local limits = M.observability_limits()
  local deadline = M.observability_deadline(now(), limits)
  local labels = { M._enabled_label }
  for _, state in ipairs(M._state_order) do
    table.insert(labels, M.state_label(state))
  end
  local rotation_seed = M.observability_rotation_seed(event)
  local issue_items, deferred_issue_pages = M.observability_list_issue_candidates(repo, labels, limits, deadline, rotation_seed)
  local pr_items, deferred_pr_pages = M.observability_list_pr_candidates(repo, limits, deadline, rotation_seed)
  local issue_numbers = M.observability_sorted_numbers(issue_items)
  local pr_numbers = M.observability_sorted_numbers(pr_items)
  local candidates, deferred_candidates = M.observability_entity_candidates(issue_numbers, pr_numbers, rotation_seed, limits.entity_cap)
  local entities = {}
  local seen_prs = {}

  local processed_issues, processed_prs, remaining_budget = observe_candidates(repo, candidates, entities, seen_prs, limits, deadline)
  if deferred_issue_pages > 0 or deferred_pr_pages > 0 or deferred_candidates > 0 or remaining_budget == 0 or not M.observability_has_budget(deadline) then
    log.warn(M.observability_deferred_log_line({
      reason = M.observability_has_budget(deadline) and "batch-cap" or "deadline",
      listed_issues = #issue_numbers,
      listed_prs = #pr_numbers,
      processed_issues = processed_issues,
      processed_prs = processed_prs,
      deferred_issues = math.max(0, #issue_numbers - processed_issues),
      deferred_prs = math.max(0, #pr_numbers - processed_prs),
      entity_cap = limits.entity_cap,
    }))
  end

  local list = {}
  for _, entity in pairs(entities) do
    entity.observability_limits = limits
    entity.observability_deadline = deadline
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
  local state_gap_report = M.state_gap_report(list)
  for _, edge in ipairs(state_gap_report.edges or {}) do
    log.info(M.state_gap_log_line(edge))
  end
  reap_orphan_prs(repo, list)
  local queue_starvation = M.observe_queue_starvation(repo, list, limits, deadline, now_seconds)
  local conflict_hotspot = M.observe_conflict_hotspots(repo, M.observability_call_timeout(limits, deadline))
  local dashboard = M.render_observability_dashboard({
    entities = list,
    counts = counts,
    stalls = stalls,
    state_gap_report = state_gap_report,
    now_seconds = now_seconds,
  })
  M.publish_observability_dashboard(repo, dashboard, limits, deadline)

  return {
    entity_count = #list,
    counts = counts,
    queue_starvation = queue_starvation,
    conflict_hotspot = conflict_hotspot,
    state_gap_report = state_gap_report,
    dashboard_hash = dashboard.hash,
  }
end
end

return S
