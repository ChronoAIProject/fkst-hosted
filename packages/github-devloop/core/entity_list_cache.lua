local S = {}

function S.install(M)
local poll_bucket_seconds = 300

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
    return string.format("\\u%04X", string.byte(char))
  end)
  return '"' .. text .. '"'
end

local function normalize_poll_key(value)
  local text = tostring(value or "")
  if text ~= "" then
    local seconds = M.iso_timestamp_epoch_seconds(text)
    if seconds ~= nil then
      return "bucket-" .. tostring(math.floor(seconds / poll_bucket_seconds))
    end
    return M.sanitize_key(text, 80):gsub("/", "-")
  end
  return "bucket-" .. tostring(math.floor(now() / poll_bucket_seconds))
end

local function list_cache_key(repo, kind, scope, poll_key)
  local selected_kind = tostring(kind or "")
  if selected_kind ~= "issue" and selected_kind ~= "pr" then
    error("github-devloop: invalid entity list kind")
  end
  return table.concat({
    "github-devloop",
    "entity-list",
    M.safe_repo(repo),
    selected_kind,
    M.sanitize_key(scope or "open", 80):gsub("/", "-"),
    normalize_poll_key(poll_key),
  }, "/")
end

local function decode_cached_list(encoded)
  local ok, decoded = pcall(json.decode, encoded or "")
  if not ok or type(decoded) ~= "table" or decoded.stdout == nil then
    return nil
  end
  return {
    stdout = tostring(decoded.stdout),
    stderr = "",
    exit_code = 0,
  }
end

local function encode_cached_list(stdout)
  return '{"stdout":' .. json_string(stdout or "") .. "}"
end

local function fetch_shared_list(repo, kind, scope, poll_key, exec_spec)
  local key = list_cache_key(repo, kind, scope, poll_key)
  local cached = decode_cached_list(cache_get(key))
  if cached ~= nil then
    return cached
  end

  local result = exec_spec()
  if type(result) == "table" and result.exit_code == 0 then
    cache_set(key, encode_cached_list(result.stdout or ""))
  end
  return result
end

function M.entity_list_cache_key(repo, kind, scope, poll_key)
  return list_cache_key(repo, kind, scope, poll_key)
end

function M.entity_list_poll_key(event)
  if type(event) == "table" then
    if event.ts ~= nil then
      return tostring(event.ts)
    end
    local payload = event.payload
    if type(payload) == "table" then
      for _, key in ipairs({ "tick", "generated_at", "ts" }) do
        if payload[key] ~= nil then
          return tostring(payload[key])
        end
      end
    end
  end
  return nil
end

function M.fetch_shared_issue_observe_list(repo, opts)
  local options = opts or {}
  local cmd = M.gh_issue_list_observe_cmd(repo)
  return fetch_shared_list(repo, "issue", "open", options.poll_key, function()
    return M.gh_exec({ cmd = cmd, timeout = options.timeout or 60 }, nil, options.exec)
  end)
end

function M.fetch_shared_pr_observe_list(repo, opts)
  local options = opts or {}
  local cmd = M.gh_pr_list_observe_cmd(repo)
  return fetch_shared_list(repo, "pr", "open", options.poll_key, function()
    return M.gh_exec({ cmd = cmd, timeout = options.timeout or 60 }, nil, options.exec)
  end)
end

function M.fetch_shared_issue_intake_list(repo, limit, opts)
  local options = opts or {}
  local cmd = M.gh_issue_list_intake_cmd(repo, limit)
  return fetch_shared_list(repo, "issue", "intake-" .. tostring(limit or 100), options.poll_key, function()
    return M.gh_exec({ cmd = cmd, timeout = options.timeout or 30 }, nil, options.exec)
  end)
end

end

return S
