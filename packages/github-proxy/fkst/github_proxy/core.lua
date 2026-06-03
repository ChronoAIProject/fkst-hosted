local M = {}

local allowed_env = {
  FKST_GITHUB_REPO = true,
  FKST_GITHUB_WRITE = true,
}

local function shell_single_quote(value)
  return "'" .. tostring(value):gsub("'", "'\\''") .. "'"
end

function M.read_env_command(name)
  if not allowed_env[name] then
    error("env name is not allowed: " .. tostring(name))
  end
  return 'printf %s "$' .. name .. '"'
end

function M.read_env(name, exec)
  local run = exec or exec_sync
  if type(run) ~= "function" then
    error("read_env requires exec_sync")
  end
  local out = run(M.read_env_command(name))
  if out.exit_code ~= 0 then
    return nil
  end
  if out.stdout == "" then
    return nil
  end
  return out.stdout
end

function M.issue_dedup_key(repo, number, updated_at)
  return tostring(repo) .. "#" .. tostring(number) .. "@" .. tostring(updated_at)
end

function M.seen_grep(key)
  return "github-proxy:seen:issue:" .. tostring(key)
end

local regex_meta = {
  ["."] = true,
  ["*"] = true,
  ["["] = true,
  ["]"] = true,
  ["^"] = true,
  ["$"] = true,
  ["\\"] = true,
  ["+"] = true,
  ["?"] = true,
  ["("] = true,
  [")"] = true,
  ["{"] = true,
  ["}"] = true,
  ["|"] = true,
}

function M.grep_escape(value)
  local out = {}
  local text = tostring(value)
  for i = 1, #text do
    local ch = text:sub(i, i)
    if regex_meta[ch] then
      table.insert(out, "\\" .. ch)
    else
      table.insert(out, ch)
    end
  end
  return table.concat(out)
end

local function hex_encode(value)
  local out = {}
  local text = tostring(value)
  for i = 1, #text do
    table.insert(out, string.format("%02x", text:byte(i)))
  end
  return table.concat(out)
end

function M.ledger_path(key)
  return ".fkst-github-proxy-ledger/seen-" .. hex_encode(key)
end

function M.comment_marker(dedup_key)
  return "<!-- fkst:github-proxy:comment:" .. tostring(dedup_key) .. " -->"
end

function M.has_marker(comments_text, dedup_key)
  if comments_text == nil or comments_text == "" then
    return false
  end
  return tostring(comments_text):find(M.comment_marker(dedup_key), 1, true) ~= nil
end

local function parse_json_string(src, pos)
  pos = pos + 1
  local out = {}
  while pos <= #src do
    local ch = src:sub(pos, pos)
    if ch == '"' then
      return table.concat(out), pos + 1
    end
    if ch == "\\" then
      local esc = src:sub(pos + 1, pos + 1)
      local mapped = ({
        ['"'] = '"',
        ["\\"] = "\\",
        ["/"] = "/",
        b = "\b",
        f = "\f",
        n = "\n",
        r = "\r",
        t = "\t",
      })[esc]
      if mapped == nil then
        if esc == "u" then
          table.insert(out, "?")
          pos = pos + 6
        else
          error("unsupported json escape: \\" .. tostring(esc))
        end
      else
        table.insert(out, mapped)
        pos = pos + 2
      end
    else
      table.insert(out, ch)
      pos = pos + 1
    end
  end
  error("unterminated json string")
end

local function skip_ws(src, pos)
  while pos <= #src and src:sub(pos, pos):match("%s") do
    pos = pos + 1
  end
  return pos
end

local parse_value

local function parse_array(src, pos)
  pos = skip_ws(src, pos + 1)
  local out = {}
  if src:sub(pos, pos) == "]" then
    return out, pos + 1
  end
  while true do
    local value
    value, pos = parse_value(src, pos)
    table.insert(out, value)
    pos = skip_ws(src, pos)
    local ch = src:sub(pos, pos)
    if ch == "]" then
      return out, pos + 1
    end
    if ch ~= "," then
      error("expected comma in json array")
    end
    pos = skip_ws(src, pos + 1)
  end
end

local function parse_object(src, pos)
  pos = skip_ws(src, pos + 1)
  local out = {}
  if src:sub(pos, pos) == "}" then
    return out, pos + 1
  end
  while true do
    if src:sub(pos, pos) ~= '"' then
      error("expected json object key")
    end
    local key
    key, pos = parse_json_string(src, pos)
    pos = skip_ws(src, pos)
    if src:sub(pos, pos) ~= ":" then
      error("expected colon after json object key")
    end
    local value
    value, pos = parse_value(src, skip_ws(src, pos + 1))
    out[key] = value
    pos = skip_ws(src, pos)
    local ch = src:sub(pos, pos)
    if ch == "}" then
      return out, pos + 1
    end
    if ch ~= "," then
      error("expected comma in json object")
    end
    pos = skip_ws(src, pos + 1)
  end
end

function parse_value(src, pos)
  pos = skip_ws(src, pos)
  local ch = src:sub(pos, pos)
  if ch == '"' then
    return parse_json_string(src, pos)
  end
  if ch == "{" then
    return parse_object(src, pos)
  end
  if ch == "[" then
    return parse_array(src, pos)
  end
  local rest = src:sub(pos)
  if rest:sub(1, 4) == "true" then
    return true, pos + 4
  end
  if rest:sub(1, 5) == "false" then
    return false, pos + 5
  end
  if rest:sub(1, 4) == "null" then
    return nil, pos + 4
  end
  local num = rest:match("^-?%d+%.?%d*")
  if num then
    return tonumber(num), pos + #num
  end
  error("unexpected json value at byte " .. tostring(pos))
end

local function decode_json(src)
  local value, pos = parse_value(src or "", 1)
  pos = skip_ws(src or "", pos)
  if pos <= #(src or "") then
    error("trailing json content")
  end
  return value
end

function M.parse_issue_list(gh_json_stdout)
  local decoded = decode_json(gh_json_stdout or "[]")
  local issues = {}
  for _, item in ipairs(decoded) do
    table.insert(issues, {
      number = item.number,
      title = item.title,
      url = item.url,
      updated_at = item.updatedAt or item.updated_at,
      state = item.state,
    })
  end
  return issues
end

function M.gh_issue_list_cmd(repo)
  return "gh issue list --repo " .. shell_single_quote(repo) .. " --json number,title,updatedAt,url,state"
end

function M.gh_issue_view_comments_cmd(repo, issue_number)
  return "gh issue view " .. shell_single_quote(issue_number)
    .. " --repo " .. shell_single_quote(repo)
    .. " --json comments"
end

function M.gh_issue_comment_cmd(repo, issue_number, body_file)
  return "gh issue comment " .. shell_single_quote(issue_number)
    .. " --repo " .. shell_single_quote(repo)
    .. " --body-file " .. shell_single_quote(body_file)
end

function M.git_ledger_commit_cmd(message, ledger_path)
  return "mkdir -p .fkst-github-proxy-ledger"
    .. " && printf '%s\\n' " .. shell_single_quote(message)
    .. " > " .. shell_single_quote(ledger_path)
    .. " && git add -- " .. shell_single_quote(ledger_path)
    .. " && git commit -m " .. shell_single_quote(message)
    .. " -- " .. shell_single_quote(ledger_path)
end

return M
