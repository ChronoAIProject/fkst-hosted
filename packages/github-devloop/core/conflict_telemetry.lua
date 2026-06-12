local S = {}

function S.install(M)
local conflict_hotspot_threshold = 3
local conflict_hotspot_window_days = 7
local conflict_hotspot_window_seconds = conflict_hotspot_window_days * 24 * 60 * 60
local max_conflict_log_bytes = 200000
local max_conflict_evidence = 8

local function is_safe_conflict_path(path)
  local text = tostring(path or "")
  return text ~= ""
    and #text <= 240
    and text:find("^/") == nil
    and text:find("%.%.", 1, true) == nil
    and text:find("[%z\r\n\t%s]") == nil
    and text:find("^[%w%._%-%/]+$") ~= nil
end

local function conflict_path_key(path)
  local key = M.sanitize_key(tostring(path or ""), false):gsub("/", "-"):gsub("%-+", "-")
  if #key > 140 then
    local suffix = "-" .. M._decimal_checksum(key)
    key = M.truncate_utf8(key, 140 - #suffix):gsub("%-+$", "") .. suffix
  end
  return key
end

local function current_conflict_timestamp()
  return os.date("!%Y-%m-%dT%H:%M:%SZ", now())
end

function M.conflict_file_paths_from_unmerged(stdout)
  local paths = {}
  local seen = {}
  for line in tostring(stdout or ""):gmatch("[^\r\n]+") do
    local path = line:match("\t(.+)$")
    if path ~= nil and is_safe_conflict_path(path) then
      if not seen[path] then
        seen[path] = true
        table.insert(paths, path)
      end
    elseif path ~= nil then
      M.log_line("warn", "fix", "unknown", "CONFLICT_FILE_SKIPPED", {
        "reason=unsafe-path",
        "path_key=" .. conflict_path_key(path),
      })
    end
  end
  table.sort(paths)
  return paths
end

function M.log_conflict_files(dept, proposal_id, pr_number, unmerged_stdout)
  local paths = M.conflict_file_paths_from_unmerged(unmerged_stdout)
  if #paths == 0 then
    M.log_line("info", dept or "fix", proposal_id, "CONFLICT_FILE", {
      "action=no-op",
      "reason=no-safe-conflict-files",
      "pr=" .. tostring(pr_number or ""),
    })
    return paths
  end
  for _, path in ipairs(paths) do
    M.log_line("info", dept or "fix", proposal_id, "CONFLICT_FILE", {
      "ts=" .. current_conflict_timestamp(),
      "conflict_file=" .. path,
      "pr=" .. tostring(pr_number or ""),
      "proposal_id=" .. tostring(proposal_id or "unknown"),
    })
  end
  return paths
end

local function parse_conflict_timestamp(text)
  local timestamp = tostring(text or ""):match("ts=(%d%d%d%d%-%d%d%-%d%dT%d%d[:%-]%d%d[:%-]%d%dZ)")
    or tostring(text or ""):match("(%d%d%d%d%-%d%d%-%d%dT%d%d:%d%d:%d%dZ)")
  local seconds = M.iso_timestamp_epoch_seconds(timestamp)
  if seconds == nil then
    return nil, nil
  end
  return timestamp, seconds
end

local function parse_conflict_log_line(line)
  local text = tostring(line or "")
  if text:find("tag=CONFLICT_FILE", 1, true) == nil then
    return nil
  end
  local timestamp, timestamp_seconds = parse_conflict_timestamp(text)
  local file = text:match("conflict_file=([^%s]+)")
  local pr = text:match("pr=(%d+)")
  local proposal_id = text:match("proposal_id=([^%s]+)") or text:match("proposal=([^%s]+)")
  if timestamp_seconds == nil
    or file == nil
    or pr == nil
    or proposal_id == nil
    or not is_safe_conflict_path(file) then
    return nil
  end
  return {
    timestamp = timestamp,
    timestamp_seconds = timestamp_seconds,
    file = file,
    pr = tonumber(pr),
    proposal_id = proposal_id,
    line = text,
  }
end

function M.parse_conflict_file_facts(log_text)
  local facts = {}
  local text = tostring(log_text or "")
  if #text > max_conflict_log_bytes then
    text = text:sub(#text - max_conflict_log_bytes + 1)
  end
  for line in text:gmatch("[^\r\n]+") do
    local fact = parse_conflict_log_line(line)
    if fact ~= nil then
      table.insert(facts, fact)
    end
  end
  return facts
end

local function conflict_hotspots(facts, threshold, now_seconds)
  local cutoff_seconds = (tonumber(now_seconds) or now()) - conflict_hotspot_window_seconds
  local by_file = {}
  for _, fact in ipairs(facts or {}) do
    if fact.timestamp_seconds ~= nil and fact.timestamp_seconds >= cutoff_seconds then
      local item = by_file[fact.file]
      if item == nil then
        item = {
          file = fact.file,
          prs = {},
          pr_seen = {},
          evidence = {},
        }
        by_file[fact.file] = item
      end
      if fact.pr ~= nil and not item.pr_seen[fact.pr] then
        item.pr_seen[fact.pr] = true
        table.insert(item.prs, fact.pr)
      end
      if #item.evidence < max_conflict_evidence then
        table.insert(item.evidence, fact)
      end
    end
  end
  local result = {}
  for _, item in pairs(by_file) do
    table.sort(item.prs)
    if #item.prs >= (threshold or conflict_hotspot_threshold) then
      table.insert(result, item)
    end
  end
  table.sort(result, function(a, b)
    if #a.prs ~= #b.prs then
      return #a.prs > #b.prs
    end
    return a.file < b.file
  end)
  return result
end

local function hotspot_title(file)
  local title = "Split conflict hotspot: " .. tostring(file)
  if #title > M._max_title_len then
    title = M.truncate_utf8(title, M._max_title_len)
  end
  return title
end

local function hotspot_body(hotspot)
  local lines = {
    "Conflict hotspot detected from structured fix-lane telemetry.",
    "",
    "File: `" .. tostring(hotspot.file) .. "`",
    "Window: " .. tostring(conflict_hotspot_window_days) .. " days",
    "Distinct PRs: " .. tostring(#hotspot.prs) .. " (" .. table.concat(hotspot.prs, ", ") .. ")",
    "",
    "Evidence:",
  }
  for _, fact in ipairs(hotspot.evidence or {}) do
    table.insert(lines, "- conflict_file=" .. tostring(fact.file)
      .. " pr=" .. tostring(fact.pr)
      .. " ts=" .. tostring(fact.timestamp or "")
      .. " proposal_id=" .. tostring(fact.proposal_id))
  end
  table.insert(lines, "")
  table.insert(lines, "Requested outcome:")
  table.insert(lines, "- Evaluate whether this file should be split or sharded to reduce recurring merge conflicts.")
  table.insert(lines, "- Feed the normal intake, consensus, implementation, and review pipeline; this patrol must not restructure code directly.")
  local body = table.concat(lines, "\n")
  if #body > M._max_body_len then
    body = M.truncate_utf8(body, M._max_body_len)
  end
  return body
end

local function hotspot_parent_comment_target(repo, hotspot)
  for _, fact in ipairs(hotspot and hotspot.evidence or {}) do
    local entity = M.parse_entity_proposal_id(fact.proposal_id)
    if entity ~= nil
      and entity.kind == "issue"
      and tostring(entity.repo or "") == tostring(repo or "")
      and entity.issue_number ~= nil then
      return {
        repo = repo,
        issue_number = entity.issue_number,
      }
    end
  end
  return nil
end

function M.build_conflict_hotspot_issue_create_request(repo, hotspot)
  local key = conflict_path_key(hotspot.file)
  return {
    schema = "github-proxy.issue-create.v1",
    repo = repo,
    title = hotspot_title(hotspot.file),
    body = hotspot_body(hotspot),
    labels = json.decode("[]"),
    dedup_key = M._dedup_key({
      "conflict-hotspot",
      tostring(repo or ""),
      key,
    }),
    parent_comment_target = hotspot_parent_comment_target(repo, hotspot),
    source_ref = {
      kind = "external",
      ref = tostring(repo or "") .. "#conflict-hotspot/" .. key,
    },
  }
end

function M.observe_conflict_hotspots(repo)
  local cmd = M.read_env("FKST_DEVLOOP_CONFLICT_LOG_CMD")
  if cmd == nil or tostring(cmd) == "" then
    log.info("github-devloop dept=observability tag=CONFLICT_HOTSPOT_PATROL action=no-op reason=log-source-unconfigured")
    return { facts = 0, hotspots = 0, raised = 0 }
  end
  local result = exec_sync({ cmd = cmd, timeout = 30 })
  if type(result) ~= "table" or result.exit_code ~= 0 then
    log.warn("github-devloop dept=observability tag=CONFLICT_HOTSPOT_PATROL action=no-op reason=log-source-failed")
    return { facts = 0, hotspots = 0, raised = 0 }
  end
  local facts = M.parse_conflict_file_facts(result.stdout)
  local hotspots = conflict_hotspots(facts, conflict_hotspot_threshold, now())
  local raised = 0
  for _, hotspot in ipairs(hotspots) do
    local request = M.build_conflict_hotspot_issue_create_request(repo, hotspot)
    M.log_raise("observability", "conflict-hotspot/" .. tostring(hotspot.file), "github-proxy.github_issue_create_request", request)
    raised = raised + 1
    log.info("github-devloop dept=observability tag=CONFLICT_HOTSPOT_PATROL"
      .. " action=raise"
      .. " conflict_file=" .. tostring(hotspot.file)
      .. " distinct_prs=" .. tostring(#hotspot.prs)
      .. " dedup_key=" .. tostring(request.dedup_key))
  end
  if raised == 0 then
    log.info("github-devloop dept=observability tag=CONFLICT_HOTSPOT_PATROL"
      .. " action=no-op"
      .. " reason=below-threshold"
      .. " facts=" .. tostring(#facts)
      .. " hotspots=" .. tostring(#hotspots))
  end
  return {
    facts = #facts,
    hotspots = #hotspots,
    raised = raised,
  }
end
end

return S
