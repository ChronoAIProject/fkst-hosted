local S = {}

function S.install(M)
local max_cache_key_segment_len = 120

local function shell_single_quote(value)
  return "'" .. tostring(value):gsub("'", "'\\''") .. "'"
end

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

local function entity_view_cache_key(repo, kind, number, updated_at)
  return "github-proxy/view/"
    .. sanitize_cache_segment(repo, true)
    .. "/"
    .. sanitize_cache_segment(kind, false)
    .. "/"
    .. sanitize_cache_segment(number, false)
    .. "/"
    .. sanitize_cache_segment(updated_at, false)
end

local function entity_view_cmd(repo, kind, number)
  if kind == "pr" then
    return "gh pr view " .. shell_single_quote(number)
      .. " --repo " .. shell_single_quote(repo)
      .. " --json headRefName,headRefOid,baseRefName,state,updatedAt,comments"
  end
  return "gh issue view " .. shell_single_quote(number)
    .. " --repo " .. shell_single_quote(repo)
    .. " --json title,body,comments,labels,state,updatedAt"
end

local function decode_cached_view(encoded)
  local ok, decoded = pcall(json.decode, encoded or "")
  if not ok or type(decoded) ~= "table" then
    return nil
  end
  if type(decoded.stdout) ~= "string" then
    return nil
  end
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

local function encode_cached_view(stdout, producer)
  return '{"producer":' .. json_string(producer)
    .. ',"stdout":' .. json_string(stdout or "")
    .. "}"
end

local function fetch_entity_view(repo, kind, number, updated_at, opts)
  local selected_kind = tostring(kind or "")
  if selected_kind ~= "issue" and selected_kind ~= "pr" then
    error("github-proxy: invalid entity view kind")
  end
  local options = opts or {}
  local cmd = entity_view_cmd(repo, selected_kind, number)
  local consumer = tostring(options.consumer or "")
  if options.fresh == true or updated_at == nil or tostring(updated_at) == "" then
    return M.gh_exec(cmd, 30, "gh " .. selected_kind .. " view")
  end

  local key = entity_view_cache_key(repo, selected_kind, number, updated_at)
  local cached = decode_cached_view(cache_get(key))
  if cached ~= nil and cached.producer ~= consumer then
    cache_set(key, "")
    return {
      stdout = cached.stdout,
      stderr = "",
      exit_code = 0,
    }
  end
  local result = M.gh_exec(cmd, 30, "gh " .. selected_kind .. " view")
  if type(result) == "table" and result.exit_code == 0 then
    cache_set(key, encode_cached_view(result.stdout or "", consumer))
  end
  return result
end

function M.entity_view_cache_key(repo, kind, number, updated_at)
  return entity_view_cache_key(repo, kind, number, updated_at)
end

function M.gh_issue_view_entity_cmd(repo, issue_number)
  return entity_view_cmd(repo, "issue", issue_number)
end

function M.gh_pr_view_entity_cmd(repo, pr_number)
  return entity_view_cmd(repo, "pr", pr_number)
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

end

return S
