local M = {}

local allowed_env = {
  FKST_GITHUB_REPO = true,
  FKST_GITHUB_WRITE = true,
  FKST_RUNTIME_ROOT = true,
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

local function hex_encode(value)
  local out = {}
  local text = tostring(value)
  for i = 1, #text do
    table.insert(out, string.format("%02x", text:byte(i)))
  end
  return table.concat(out)
end

function M.seen_marker_path(runtime_root, key)
  return tostring(runtime_root) .. "/github-proxy/seen/" .. hex_encode(key)
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

-- Decodes gh --json output via the engine-provided json.decode; requires a json-capable substrate runtime.
function M.parse_issue_list(gh_json_stdout)
  local decoded = json.decode(gh_json_stdout or "[]")
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

function M.mkdir_p_cmd(dir)
  return "mkdir -p " .. shell_single_quote(dir)
end

return M
