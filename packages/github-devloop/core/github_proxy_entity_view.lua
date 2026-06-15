local S = {}

function S.install(M)
local max_cache_key_segment_len = 120

local function sanitize_cache_segment(value, allow_slash)
  local pattern = allow_slash and "[^%w%._%-%/]" or "[^%w%._%-]"
  local safe = tostring(value or ""):gsub(pattern, "-")
  safe = safe:gsub("-+", "-")
  if allow_slash then
    safe = safe:gsub("/+", "/"):gsub("^/+", ""):gsub("/+$", "")
  else
    safe = safe:gsub("^-+", ""):gsub("-+$", "")
  end
  local segments = {}
  for segment in safe:gmatch("[^/]+") do
    if segment == "." or segment == ".." then
      segment = "-"
    end
    table.insert(segments, segment)
  end
  safe = table.concat(segments, allow_slash and "/" or "-")
  if #safe > max_cache_key_segment_len then
    safe = safe:sub(1, max_cache_key_segment_len):gsub("/+$", ""):gsub("-+$", "")
  end
  if safe == "" then
    return "empty"
  end
  return safe
end

local function entity_view_cache_key(repo, kind, number)
  return "github-proxy/view/"
    .. sanitize_cache_segment(repo, true)
    .. "/"
    .. sanitize_cache_segment(kind, false)
    .. "/"
    .. sanitize_cache_segment(number, false)
end

local function issue_view_cmd(repo, issue_number)
  return "gh issue view " .. M._shell_single_quote(issue_number)
    .. " --repo " .. M._shell_single_quote(repo)
    .. " --json title,body,comments,labels,state,updatedAt,assignees,author"
end

local function pr_view_cmd(repo, pr_number)
  return "gh pr view " .. M._shell_single_quote(pr_number)
    .. " --repo " .. M._shell_single_quote(repo)
    .. " --json headRefName,headRefOid,baseRefName,state,updatedAt,comments,labels"
end

local function entity_updated_at_cmd(repo, kind, number)
  local path_kind = kind == "pr" and "pulls" or "issues"
  return "gh api "
    .. M._shell_single_quote("repos/" .. tostring(repo) .. "/" .. path_kind .. "/" .. tostring(number))
    .. " --jq " .. M._shell_single_quote(".updated_at // .updatedAt // \"\"")
end

local function entity_view_cmd(repo, kind, number)
  if kind == "pr" then
    return pr_view_cmd(repo, number)
  end
  return issue_view_cmd(repo, number)
end

local function parse_view_updated_at(stdout)
  local ok, decoded = pcall(json.decode, stdout or "")
  if not ok or type(decoded) ~= "table" then
    return nil
  end
  local updated_at = decoded.updatedAt or decoded.updated_at
  if updated_at == nil or tostring(updated_at) == "" then
    return nil
  end
  return tostring(updated_at)
end

local function parse_updated_at_stdout(stdout)
  local text = tostring(stdout or "")
  text = text:gsub("^%s+", ""):gsub("%s+$", "")
  if text == "" then
    return nil
  end
  return text
end

local function decode_cached_view(encoded)
  local ok, decoded = pcall(json.decode, encoded or "")
  if not ok or type(decoded) ~= "table" then
    return nil
  end
  if type(decoded.stdout) ~= "string" then
    return nil
  end
  if decoded.updated_at == nil or tostring(decoded.updated_at) == "" then
    return nil
  end
  decoded.updated_at = tostring(decoded.updated_at)
  return decoded
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
  return '"' .. text .. '"'
end

local function encode_cached_view(stdout, updated_at, producer)
  return '{"updated_at":' .. json_string(updated_at)
    .. ',"producer":' .. json_string(producer)
    .. ',"stdout":' .. json_string(stdout or "")
    .. "}"
end

local function success_from_cache(cached)
  return {
    stdout = cached.stdout,
    stderr = "",
    exit_code = 0,
  }
end

local function cache_successful_view(key, result, producer)
  local view_updated_at = parse_view_updated_at(result and result.stdout)
  if type(result) == "table" and result.exit_code == 0 and view_updated_at ~= nil then
    cache_set(key, encode_cached_view(result.stdout or "", view_updated_at, producer))
  end
end

local function fetch_entity_view(repo, kind, number, updated_at, opts)
  local selected_kind = tostring(kind or "")
  if selected_kind ~= "issue" and selected_kind ~= "pr" then
    error("github-devloop: invalid entity view kind")
  end

  local cmd = entity_view_cmd(repo, selected_kind, number)
  local validator = tostring(updated_at or "")
  local options = opts or {}
  local consumer = tostring(options.consumer or "")
  local timeout = tonumber(options.timeout) or 30
  local key = entity_view_cache_key(repo, selected_kind, number)
  if options.force_fresh == true then
    local result = M.gh_exec(cmd, timeout)
    cache_successful_view(key, result, consumer)
    return result
  end

  local cached = decode_cached_view(cache_get(key))
  if validator ~= "" then
    -- GitHub updatedAt is second-granular, so this validator is freshness-best-effort only.
    -- It is safe for stale-tolerant observe reads; authority and write-gate reads must force_fresh.
    if cached ~= nil and cached.updated_at == validator then
      return success_from_cache(cached)
    end
    local result = M.gh_exec(cmd, timeout)
    cache_successful_view(key, result, consumer)
    return result
  end

  if cached ~= nil then
    local current = M.gh_exec(entity_updated_at_cmd(repo, selected_kind, number), timeout)
    if current.exit_code ~= 0 then
      return current
    end
    if parse_updated_at_stdout(current.stdout) == cached.updated_at then
      return success_from_cache(cached)
    end
    cache_set(key, "")
  end

  local result = M.gh_exec(cmd, timeout)
  cache_successful_view(key, result, consumer)
  return result
end

function M.entity_view_cache_key(repo, kind, number)
  return entity_view_cache_key(repo, kind, number)
end

function M.entity_cache_key(repo, entity_type, number)
  return "github-proxy/" .. tostring(entity_type) .. "/" .. tostring(repo) .. "/" .. tostring(number)
end

function M.invalidate_entity_after_write(repo, kind, number)
  local selected_kind = tostring(kind or "")
  if selected_kind ~= "issue" and selected_kind ~= "pr" then
    error("github-devloop: invalid post-write invalidation kind")
  end
  local entity_key = M.entity_cache_key(repo, selected_kind, number)
  local view_key = entity_view_cache_key(repo, selected_kind, number)
  with_lock(entity_key, function()
    cache_set(entity_key, "")
    cache_set(view_key, "")
  end)
end

function M.gh_issue_view_entity_cmd(repo, issue_number)
  return issue_view_cmd(repo, issue_number)
end

function M.gh_pr_view_entity_cmd(repo, pr_number)
  return pr_view_cmd(repo, pr_number)
end

function M.gh_entity_updated_at_cmd(repo, kind, number)
  local selected_kind = tostring(kind or "")
  if selected_kind ~= "issue" and selected_kind ~= "pr" then
    error("github-devloop: invalid entity updatedAt kind")
  end
  return entity_updated_at_cmd(repo, selected_kind, number)
end

function M.fetch_entity_view(repo, kind, number, updated_at, opts)
  return fetch_entity_view(repo, kind, number, updated_at, opts)
end

function M.fetch_issue_view(repo, issue_number, updated_at, opts)
  return fetch_entity_view(repo, "issue", issue_number, updated_at, opts)
end

function M.fetch_pr_view(repo, pr_number, updated_at, opts)
  return fetch_entity_view(repo, "pr", pr_number, updated_at, opts)
end

function M.fetch_marker_issue_view(repo, issue_number, updated_at, opts)
  local options = opts or {}
  options.consumer = options.consumer or "marker-reader"
  return M.fetch_issue_view(repo, issue_number, updated_at, options)
end

function M.fetch_marker_pr_view(repo, pr_number, updated_at, opts)
  local options = opts or {}
  options.consumer = options.consumer or "marker-reader"
  return M.fetch_pr_view(repo, pr_number, updated_at, options)
end

function M.fetch_issue_view_state(repo, issue_number, updated_at, opts)
  local options = opts or {}
  options.consumer = options.consumer or "observe_issue"
  return M.fetch_marker_issue_view(repo, issue_number, updated_at, options)
end

function M.fetch_issue_view_open_pr(repo, issue_number, updated_at, opts)
  local options = opts or {}
  options.consumer = options.consumer or "open_pr"
  return M.fetch_marker_issue_view(repo, issue_number, updated_at, options)
end

function M.commit_issue_subject_snapshot(repo, issue_number)
  if issue_number == nil then
    return {}
  end
  local ok, view = pcall(M.gh_exec, { cmd = M.gh_issue_view_commit_subject_cmd(repo, issue_number), timeout = 30 })
  if not ok or type(view) ~= "table" or view.exit_code ~= 0 then
    return {}
  end
  local decoded_ok, decoded = pcall(json.decode, view.stdout or "{}")
  if not decoded_ok or type(decoded) ~= "table" then
    return {}
  end
  return {
    title = tostring(decoded.title or ""),
  }
end

function M.fetch_pr_view_origin(repo, pr_number, updated_at, opts)
  local options = opts or {}
  options.consumer = options.consumer or "observe_pr"
  return M.fetch_marker_pr_view(repo, pr_number, updated_at, options)
end

end

return S
