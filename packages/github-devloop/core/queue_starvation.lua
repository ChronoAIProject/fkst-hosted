local S = {}

function S.install(M)
local detector = "queue-starvation"
local merge_recent_threshold_minutes = 360
local merge_ready_stale_threshold_minutes = 60
local recent_closed_limit = 30

local function format_timestamp(seconds)
  return os.date("!%Y-%m-%dT%H:%M:%SZ", tonumber(seconds) or now())
end

local function snapshot_path(repo, window_key)
  local safe_repo = M.safe_repo(repo):gsub("/", "-"):gsub("%-+", "-")
  local safe_window = M.sanitize_key(tostring(window_key or "unknown"), false):gsub("[/%s]+", "-")
  safe_window = safe_window:gsub("[^%w%._%-]", "-"):gsub("%-+", "-"):gsub("^%-+", ""):gsub("%-+$", "")
  if safe_window == "" then
    safe_window = "unknown"
  end
  if #safe_window > 120 then
    safe_window = safe_window:sub(1, 120):gsub("%-+$", "")
  end
  return "/tmp/fkst-github-devloop-queue-starvation-" .. safe_repo .. "-" .. safe_window .. ".json"
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

local function json_array(values)
  local parts = {}
  for _, value in ipairs(values or {}) do
    table.insert(parts, json_string(value))
  end
  return "[" .. table.concat(parts, ",") .. "]"
end

local function issue_json(issue)
  return "{"
    .. '"number":' .. tostring(tonumber(issue.number) or 0)
    .. ',"title":' .. json_string(issue.title)
    .. ',"closedAt":' .. json_string(issue.closed_at)
    .. ',"labels":' .. json_array(issue.labels)
    .. "}"
end

local function entity_json(entity, age_minutes)
  return "{"
    .. '"proposal_id":' .. json_string(entity and entity.proposal_id or "")
    .. ',"issue_number":' .. json_string(entity and entity.issue_number or "")
    .. ',"pr_number":' .. json_string(entity and entity.pr_number or "")
    .. ',"title":' .. json_string(entity and entity.title or "")
    .. ',"state":' .. json_string(entity and entity.state and entity.state.state or "")
    .. ',"version":' .. json_string(entity and entity.state and entity.state.version or "")
    .. ',"head_sha":' .. json_string(entity and entity.head_sha or "")
    .. ',"source":' .. json_string(entity and entity.source or "")
    .. ',"age_minutes":' .. tostring(tonumber(age_minutes) or 0)
    .. "}"
end

local function write_snapshot(repo, window_key, evidence)
  local path = snapshot_path(repo, window_key)
  local closed = {}
  for _, issue in ipairs(evidence.recent_closed or {}) do
    table.insert(closed, issue_json(issue))
  end
  file.write(path, "{"
    .. '"detector":' .. json_string(detector)
    .. ',"repo":' .. json_string(repo)
    .. ',"window":' .. json_string(window_key)
    .. ',"generated_at":' .. json_string(format_timestamp(evidence.now_seconds))
    .. ',"queue_head":' .. entity_json(evidence.queue_head, evidence.queue_head_age_minutes)
    .. ',"threshold_minutes":' .. tostring(evidence.threshold_minutes)
    .. ',"last_merge_age_minutes":' .. json_string(evidence.last_merge_age_minutes or "none")
    .. ',"recent_closed":[' .. table.concat(closed, ",") .. "]"
    .. "}\n")
  return path
end

local function marker_attr(marker, name)
  return marker:match(name .. '="([^"]*)"')
end

local function merged_fact_from_issue(issue)
  for _, comment in ipairs(M._trusted_marker_comments(issue and issue.comments or {})) do
    for marker in M._comment_body(comment):gmatch("<!%-%- fkst:github%-devloop:merged:v1.-%-%->") do
      local proposal_id = marker_attr(marker, "proposal")
      local pr_number = marker_attr(marker, "pr")
      local version = marker_attr(marker, "version")
      local head_sha = marker_attr(marker, "head_sha")
      if M._is_bounded_string(proposal_id, M._max_key_len)
        and M._is_positive_pr_number(pr_number)
        and M._is_bounded_string(version, M._max_dedup_len)
        and M._is_git_sha(head_sha) then
        return {
          proposal_id = proposal_id,
          pr_number = tonumber(pr_number),
          version = version,
          head_sha = head_sha,
          closed_at = issue.closed_at,
          issue_number = issue.number,
          comment_created_at = M._comment_created_at(comment),
        }
      end
    end
  end
  return nil
end

local function has_label(issue, label)
  for _, item in ipairs(issue and issue.labels or {}) do
    if item == label then
      return true
    end
  end
  return false
end

function M.queue_starvation_recent_closed_merged_issues(repo, limits, deadline)
  local listed = M.observability_run_cmd(
    M.gh_issue_list_recent_closed_cmd(repo, recent_closed_limit),
    limits,
    deadline,
    "gh recent closed merged issue list"
  )
  local issues = M.parse_issue_list_recent_closed(listed.stdout)
  local merged = {}
  for _, issue in ipairs(issues) do
    local fact = nil
    if has_label(issue, M._merged_label) then
      local view = M.observability_run_cmd(
        M.gh_issue_view_observe_cmd(repo, issue.number),
        limits,
        deadline,
        "gh recent closed merged issue view"
      )
      local current = M.parse_issue_view_observe(view.stdout)
      current.closed_at = issue.closed_at
      current.number = issue.number
      fact = merged_fact_from_issue(current)
    end
    if fact ~= nil then
      table.insert(merged, {
        number = issue.number,
        title = issue.title,
        closed_at = issue.closed_at,
        labels = issue.labels,
        merged = fact,
      })
    end
  end
  return issues, merged
end

local function age_minutes(timestamp, now_seconds)
  local seconds = M.iso_timestamp_epoch_seconds(timestamp)
  if seconds == nil then
    return nil
  end
  local age = (tonumber(now_seconds) or now()) - seconds
  if age < 0 then
    return nil
  end
  return math.floor(age / 60)
end

local function newest_recent_merge(merged, now_seconds)
  local newest = nil
  for _, issue in ipairs(merged or {}) do
    local age = age_minutes(issue.closed_at, now_seconds)
    if age ~= nil and (newest == nil or age < newest.age_minutes) then
      newest = {
        issue = issue,
        age_minutes = age,
      }
    end
  end
  return newest
end

local function merge_ready_queue_head(entities, now_seconds)
  local selected = nil
  for _, entity in ipairs(entities or {}) do
    local state = entity.state and entity.state.state or nil
    local age = M.stall_suspect_age_minutes(entity.state and entity.state.version or nil, now_seconds)
    if state == "merge-ready"
      and tonumber(age) ~= nil
      and tonumber(age) > merge_ready_stale_threshold_minutes
      and (selected == nil
        or tonumber(age) > tonumber(selected.age_minutes)
        or (tonumber(age) == tonumber(selected.age_minutes)
          and tostring(entity.proposal_id or "") < tostring(selected.entity and selected.entity.proposal_id or ""))) then
      selected = {
        entity = entity,
        state = state,
        age_minutes = age,
        threshold_minutes = merge_ready_stale_threshold_minutes,
      }
    end
  end
  return selected
end

local function merge_queue_head_entity(repo, now_seconds)
  local branches = M.branch_config()
  local head = M.merge_queue_head(repo, branches.integration)
  if head == nil or tostring(head.state or "") ~= "merge-ready" then
    return nil
  end
  local repo_from_proposal, issue_number = M.parse_proposal_id(head.proposal_id)
  if repo_from_proposal == nil then
    return nil
  end
  local age = M.stall_suspect_age_minutes(head.version, now_seconds)
  if tonumber(age) == nil or tonumber(age) <= merge_ready_stale_threshold_minutes then
    return nil
  end
  return {
    entity = {
      proposal_id = head.proposal_id,
      issue_number = tonumber(issue_number) or issue_number,
      pr_number = head.pr_number,
      title = "PR #" .. tostring(head.pr_number),
      state = {
        state = "merge-ready",
        version = head.version,
      },
      head_sha = head.head_sha,
      source = "merge-queue",
    },
    state = "merge-ready",
    age_minutes = age,
    threshold_minutes = merge_ready_stale_threshold_minutes,
  }
end

local function observed_queue_head(entities, now_seconds)
  local selected = merge_ready_queue_head(entities, now_seconds)
  if selected ~= nil and selected.entity ~= nil then
    selected.entity.source = selected.entity.source or "observability-sample"
  end
  return selected
end

local function queue_head_for_starvation(repo, entities, now_seconds)
  local ok, head = pcall(function()
    return merge_queue_head_entity(repo, now_seconds)
  end)
  if ok and head ~= nil then
    return head
  end
  if not ok then
    log.warn("github-devloop dept=observability tag=QUEUE_STARVATION action=fallback reason=merge-queue-source-failed")
  end
  return observed_queue_head(entities, now_seconds)
end

function M.queue_starvation_window_key(now_seconds)
  local bucket_seconds = merge_recent_threshold_minutes * 60
  local bucket = math.floor((tonumber(now_seconds) or now()) / bucket_seconds)
  return "window-" .. tostring(bucket)
end

local function stable_incident_identity(queue_head)
  local entity = queue_head and queue_head.entity or queue_head
  if type(entity) ~= "table" then
    return "merge-ready"
  end
  local parts = { "merge-ready" }
  if entity.pr_number ~= nil then
    table.insert(parts, "pr")
    table.insert(parts, tostring(entity.pr_number))
  end
  if entity.proposal_id ~= nil then
    table.insert(parts, "proposal")
    table.insert(parts, tostring(entity.proposal_id))
  end
  local version = entity.state and entity.state.version or entity.version
  if version ~= nil then
    table.insert(parts, "version")
    table.insert(parts, M.safe_version_segment(version))
  end
  if entity.head_sha ~= nil and M._is_git_sha(entity.head_sha) then
    table.insert(parts, "head")
    table.insert(parts, M.safe_head_segment(entity.head_sha))
  end
  return table.concat(parts, "/")
end

function M.queue_starvation_dedup_key(repo, identity)
  return M._dedup_key({
    detector,
    tostring(repo or ""),
    tostring(identity or "queue"),
  })
end

local function alert_title(queue_head)
  local issue = queue_head and queue_head.issue_number or "unknown"
  return "Queue starvation: merge-ready head #" .. tostring(issue) .. " has no recent merge"
end

local function alert_body(evidence, snapshot)
  local head = evidence.queue_head or {}
  local lines = {
    "Queue starvation watchdog fired from deterministic observability signals.",
    "",
    "Detector: `" .. detector .. "`",
    "Queue head: #" .. tostring(head.issue_number or "unknown") .. " " .. M.neutralize_untrusted_comment_text(head.title or ""),
    "Queue head PR: #" .. tostring(head.pr_number or "unknown"),
    "Queue head age: " .. tostring(evidence.queue_head_age_minutes) .. " minutes",
    "Threshold: " .. tostring(evidence.threshold_minutes) .. " minutes",
    "Last merge age: " .. tostring(evidence.last_merge_age_minutes or "none"),
    "Head source: `" .. tostring(head.source or "unknown") .. "`",
    "Evidence snapshot: `" .. tostring(snapshot) .. "`",
    "",
    "Requested outcome:",
    "- Diagnose why merge-ready work is not making forward progress.",
    "- File any fix through the normal intake, consensus, implementation, and review pipeline.",
    "- This watchdog must not repair, merge, relabel, or mutate runtime state directly.",
  }
  local body = table.concat(lines, "\n")
  if #body > M._max_body_len then
    body = M.truncate_utf8(body, M._max_body_len)
  end
  return body
end

function M.build_queue_starvation_issue_create_request(repo, evidence, snapshot)
  local identity = evidence.incident_identity or stable_incident_identity(evidence.queue_head)
  local window_key = evidence.window_key
  return {
    schema = "github-proxy.issue-create.v1",
    repo = repo,
    title = alert_title(evidence.queue_head),
    body = alert_body(evidence, snapshot),
    labels = json.decode("[]"),
    dedup_key = M.queue_starvation_dedup_key(repo, identity),
    parent_comment_target = {
      repo = repo,
      issue_number = tostring(evidence.queue_head.issue_number),
    },
    source_ref = {
      kind = "external",
      ref = tostring(repo or "") .. "#" .. detector .. "/" .. identity .. "/" .. tostring(window_key),
    },
  }
end

function M.observe_queue_starvation(repo, entities, limits, deadline, now_seconds)
  local queue_head = queue_head_for_starvation(repo, entities, now_seconds)
  if queue_head == nil then
    log.info("github-devloop dept=observability tag=QUEUE_STARVATION action=no-op reason=no-stale-merge-ready")
    return { action = "no-op", reason = "no-stale-merge-ready" }
  end

  local ok, recent_closed, merged = pcall(function()
    local issues, merged_issues = M.queue_starvation_recent_closed_merged_issues(repo, limits, deadline)
    return issues, merged_issues
  end)
  if not ok then
    log.warn("github-devloop dept=observability tag=QUEUE_STARVATION action=no-op reason=recent-merge-source-failed")
    return { action = "no-op", reason = "recent-merge-source-failed" }
  end

  local current_seconds = tonumber(now_seconds) or now()
  local newest = newest_recent_merge(merged, current_seconds)
  if newest ~= nil and newest.age_minutes <= merge_recent_threshold_minutes then
    log.info("github-devloop dept=observability tag=QUEUE_STARVATION action=suppress"
      .. " reason=recent-merge"
      .. " last_merge_age_minutes=" .. tostring(newest.age_minutes)
      .. " threshold_minutes=" .. tostring(merge_recent_threshold_minutes))
    return { action = "suppress", reason = "recent-merge", last_merge_age_minutes = newest.age_minutes }
  end

  local evidence = {
    now_seconds = current_seconds,
    window_key = M.queue_starvation_window_key(current_seconds),
    queue_head = queue_head.entity,
    queue_head_age_minutes = queue_head.age_minutes,
    threshold_minutes = merge_ready_stale_threshold_minutes,
    last_merge_age_minutes = newest and newest.age_minutes or nil,
    recent_closed = recent_closed,
  }
  evidence.incident_identity = stable_incident_identity(queue_head)
  local snapshot = write_snapshot(repo, evidence.window_key, evidence)
  local request = M.build_queue_starvation_issue_create_request(repo, evidence, snapshot)
  local redrive = nil
  if queue_head.entity ~= nil and queue_head.entity.pr_number ~= nil then
    redrive = M.merge_queue_starvation_tick_payload(repo, evidence.incident_identity, {
      pr_number = queue_head.entity.pr_number,
      proposal_id = queue_head.entity.proposal_id,
      version = queue_head.entity.state and queue_head.entity.state.version or nil,
      head_sha = queue_head.entity.head_sha,
    }, evidence.window_key)
  end
  M.log_raise("observability", detector .. "/merge-ready", "github-proxy.github_issue_create_request", request)
  if redrive ~= nil then
    M.log_raise("observability", detector .. "/merge-ready", "devloop_merge_queue_tick", redrive)
    raise("devloop_merge_queue_tick", redrive)
  end
  log.info("github-devloop dept=observability tag=QUEUE_STARVATION"
    .. " action=raise"
    .. " queue_head=" .. tostring(queue_head.entity and queue_head.entity.proposal_id or "")
    .. " age_minutes=" .. tostring(queue_head.age_minutes)
    .. " threshold_minutes=" .. tostring(merge_ready_stale_threshold_minutes)
    .. " last_merge_age_minutes=" .. tostring(evidence.last_merge_age_minutes or "none")
    .. " snapshot_path=" .. tostring(snapshot)
    .. " dedup_key=" .. tostring(request.dedup_key))
  return {
    action = "raise",
    request = request,
    redrive = redrive,
    snapshot_path = snapshot,
  }
end
end

return S
