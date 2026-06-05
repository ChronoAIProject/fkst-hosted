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

function M.entity_cache_key(repo, entity_type, number)
  return "github-proxy/" .. tostring(entity_type) .. "/" .. tostring(repo) .. "/" .. tostring(number)
end

function M.entity_dedup_key(repo, entity_type, number, updated_at)
  return tostring(repo)
    .. "#"
    .. tostring(entity_type)
    .. "#"
    .. tostring(number)
    .. "@"
    .. tostring(updated_at)
end

function M.issue_dedup_key(repo, number, updated_at)
  return M.entity_dedup_key(repo, "issue", number, updated_at)
end

-- Stable source pointer for the durable-delivery engine: a reliable consumer
-- re-derives the current entity from this ref (e.g. `gh issue view`) instead of
-- trusting a possibly-stale payload. ref is the entity identity WITHOUT the
-- version (updated_at lives in dedup_key / the payload).
function M.entity_source_ref(repo, entity_type, number)
  return {
    kind = "external",
    ref = tostring(repo) .. "#" .. tostring(entity_type) .. "/" .. tostring(number),
  }
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
function M.parse_entity_list(gh_json_stdout)
  local decoded = json.decode(gh_json_stdout or "[]")
  local entities = {}
  for _, item in ipairs(decoded) do
    local labels = {}
    for _, label in ipairs(item.labels or {}) do
      if type(label) == "table" and label.name ~= nil then
        table.insert(labels, tostring(label.name))
      elseif type(label) == "string" then
        table.insert(labels, label)
      end
    end
    table.insert(entities, {
      number = item.number,
      title = item.title,
      url = item.url,
      updated_at = item.updatedAt or item.updated_at,
      state = item.state,
      labels = labels,
    })
  end
  return entities
end

function M.parse_issue_list(gh_json_stdout)
  return M.parse_entity_list(gh_json_stdout)
end

function M.gh_issue_list_cmd(repo)
  return "gh issue list --repo " .. shell_single_quote(repo) .. " --state all --json number,title,updatedAt,url,state,labels"
end

function M.gh_pr_list_cmd(repo)
  return "gh pr list --repo " .. shell_single_quote(repo) .. " --state all --json number,title,updatedAt,url,state,labels"
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

function M.gh_issue_edit_labels_cmd(repo, issue_number, add_labels, remove_labels)
  local cmd = "gh issue edit " .. shell_single_quote(issue_number)
    .. " --repo " .. shell_single_quote(repo)
  for _, label in ipairs(add_labels or {}) do
    cmd = cmd .. " --add-label " .. shell_single_quote(label)
  end
  for _, label in ipairs(remove_labels or {}) do
    cmd = cmd .. " --remove-label " .. shell_single_quote(label)
  end
  return cmd
end

return M
