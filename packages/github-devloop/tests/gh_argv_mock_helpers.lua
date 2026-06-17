local M = {}

local function unique(values)
  local out = {}
  local seen = {}
  for _, value in ipairs(values or {}) do
    if type(value) == "string" and value ~= "" and not seen[value] then
      table.insert(out, value)
      seen[value] = true
    end
  end
  return out
end

local function strip_simple_shell_quotes(command)
  local stripped = tostring(command or ""):gsub("'([^']*)'", "%1")
  return stripped
end

function M.argv_rendered(command)
  return strip_simple_shell_quotes(command)
end

function M.count_calls(t, needle)
  local count = 0
  local text = tostring(needle or "")
  local unquoted = strip_simple_shell_quotes(text)
  for _, call in ipairs(t.command_calls()) do
    local rendered = tostring(call.rendered or "")
    local unquoted_rendered = strip_simple_shell_quotes(rendered)
    if rendered:find(text, 1, true)
      or rendered:find(unquoted, 1, true)
      or unquoted_rendered:find(text, 1, true)
      or unquoted_rendered:find(unquoted, 1, true) then
      count = count + 1
    end
  end
  return count
end

local function append_gh_mock_patterns(patterns, command)
  local text = tostring(command or "")
  if text:find("gh ", 1, true) == nil then
    return
  end
  table.insert(patterns, strip_simple_shell_quotes(text))
  local issue_number, repo, issue_fields = text:match("^gh issue view '([^']+)' %-%-repo '([^']+)' %-%-json ([^ ]+)$")
  if issue_number ~= nil then
    table.insert(patterns, "gh issue view " .. issue_number .. " --repo " .. repo .. " --json '" .. issue_fields .. "'")
    return
  end
  local pr_number, pr_repo, pr_fields = text:match("^gh pr view '([^']+)' %-%-repo '([^']+)' %-%-json ([^ ]+)$")
  if pr_number ~= nil then
    table.insert(patterns, "gh pr view " .. pr_number .. " --repo " .. pr_repo .. " --json '" .. pr_fields .. "'")
    return
  end
  local edit_number, edit_repo = text:match("^gh issue edit '([^']+)' %-%-repo '([^']+)' ")
  if edit_number ~= nil then
    table.insert(patterns, "gh issue edit " .. edit_number .. " --repo " .. edit_repo)
  end
  local list_repo, list_state, list_limit, list_fields = text:match("^gh issue list %-%-repo '([^']+)' %-%-state ([^ ]+) %-%-limit ([^ ]+) %-%-json ([^ ]+)$")
  if list_repo ~= nil then
    table.insert(patterns, "gh issue list --repo " .. list_repo .. " --state " .. list_state .. " --limit " .. list_limit .. " --json '" .. list_fields .. "'")
  end
  local pr_list_repo, pr_list_state, pr_list_limit, pr_list_fields = text:match("^gh pr list %-%-repo '([^']+)' %-%-state ([^ ]+) %-%-limit ([^ ]+) %-%-json ([^ ]+)$")
  if pr_list_repo ~= nil then
    table.insert(patterns, "gh pr list --repo " .. pr_list_repo .. " --state " .. pr_list_state .. " --limit " .. pr_list_limit .. " --json '" .. pr_list_fields .. "'")
  end
  local api_path = text:match("^gh api '([^']+)'$")
  if api_path ~= nil then
    table.insert(patterns, "gh api " .. api_path)
  end
  local comments_path = text:match("^gh api %-%-paginate %-%-slurp '([^']+)'$")
  if comments_path ~= nil then
    table.insert(patterns, "gh api --paginate --slurp " .. comments_path)
  end
  local jq_path, jq_expr = text:match("^gh api '([^']+)' %-%-jq '([^']+)'$")
  if jq_path ~= nil then
    table.insert(patterns, "gh api " .. jq_path .. " --jq " .. jq_expr)
    table.insert(patterns, "gh api " .. jq_path .. " --jq '" .. jq_expr .. "'")
  end
  local method, method_path = text:match("^gh api %-%-method ([^ ]+) '([^']+)'$")
  if method_path ~= nil then
    table.insert(patterns, "gh api --method " .. method .. " " .. method_path)
  end
end

local function install_command_shim(t)
  if t._gh_argv_mock_shim_installed == true then
    return
  end
  local raw_mock_command = t.mock_command
  t.mock_command = function(command, result)
    local patterns = { command }
    append_gh_mock_patterns(patterns, command)
    for _, pattern in ipairs(unique(patterns)) do
      raw_mock_command(pattern, result)
    end
  end
  t._gh_argv_mock_shim_installed = true
end

local function gh_issue_view_entity_command(repo, issue_number)
  return "gh issue view " .. tostring(issue_number)
    .. " --repo " .. tostring(repo)
    .. " --json"
end

local function gh_pr_view_entity_command(repo, pr_number)
  return "gh pr view " .. tostring(pr_number)
    .. " --repo " .. tostring(repo)
    .. " --json"
end

local function gh_entity_updated_at_command(repo, kind, number)
  local path_kind = kind == "pr" and "pulls" or "issues"
  return "gh api " .. "repos/" .. tostring(repo) .. "/" .. path_kind .. "/" .. tostring(number)
    .. " --jq .updated_at // .updatedAt // \"\""
end

function M.install(t, core)
  install_command_shim(t)
  core.gh_issue_view_entity_cmd = core.gh_issue_view_entity_cmd or gh_issue_view_entity_command
  core.gh_pr_view_entity_cmd = core.gh_pr_view_entity_cmd or gh_pr_view_entity_command
  core.gh_entity_updated_at_cmd = core.gh_entity_updated_at_cmd or gh_entity_updated_at_command
end

return M
