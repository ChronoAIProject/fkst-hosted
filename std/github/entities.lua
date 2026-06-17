local M = {}

local function url_encode(value)
  return (tostring(value or ""):gsub("([^%w%-%._~])", function(char)
    return string.format("%%%02X", string.byte(char))
  end))
end

local function repo_owner(repo)
  return tostring(repo or ""):match("^([^/]+)/")
end

local function issue_list_argv(repo)
  return { "gh", "api", "--paginate", "--slurp", "repos/" .. tostring(repo) .. "/issues?state=open&per_page=100" }
end

local function issue_list_cli_argv(repo, state, limit, fields)
  return {
    "gh",
    "issue",
    "list",
    "--repo",
    tostring(repo),
    "--state",
    tostring(state),
    "--limit",
    tostring(limit),
    "--json",
    tostring(fields),
  }
end

local function pr_list_cli_argv(repo, state, limit, fields)
  return {
    "gh",
    "pr",
    "list",
    "--repo",
    tostring(repo),
    "--state",
    tostring(state),
    "--limit",
    tostring(limit),
    "--json",
    tostring(fields),
  }
end

local function pr_list_argv(repo)
  return { "gh", "api", "--paginate", "--slurp", "repos/" .. tostring(repo) .. "/pulls?state=open&per_page=100" }
end

local function pr_list_head_argv(repo, branch, base_branch)
  local owner = repo_owner(repo)
  local head_filter = owner ~= nil and (owner .. ":" .. tostring(branch)) or tostring(branch)
  local query = "repos/" .. tostring(repo) .. "/pulls?state=open&head=" .. url_encode(head_filter) .. "&per_page=100"
  if base_branch ~= nil then
    query = query .. "&base=" .. url_encode(base_branch)
  end
  return { "gh", "api", "--paginate", "--slurp", query }
end

local function pr_view_argv(repo, pr_number)
  return { "gh", "api", "repos/" .. tostring(repo) .. "/pulls/" .. tostring(pr_number) }
end

local function entity_updated_at_argv(repo, kind, number)
  local path_kind = kind == "pr" and "pulls" or "issues"
  return {
    "gh",
    "api",
    "repos/" .. tostring(repo) .. "/" .. path_kind .. "/" .. tostring(number),
    "--jq",
    ".updated_at // .updatedAt // \"\"",
  }
end

local function issue_search_argv(repo, query, fields)
  return {
    "gh",
    "issue",
    "list",
    "--repo",
    tostring(repo),
    "--state",
    "all",
    "--limit",
    "100",
    "--search",
    tostring(query),
    "--json",
    tostring(fields),
  }
end

local function issue_create_argv(repo, title, body_file, labels, assignees)
  local argv = {
    "gh",
    "issue",
    "create",
    "--repo",
    tostring(repo),
    "--title",
    tostring(title),
    "--body-file",
    tostring(body_file),
  }
  for _, label in ipairs(labels or {}) do
    table.insert(argv, "--label")
    table.insert(argv, tostring(label))
  end
  for _, assignee in ipairs(assignees or {}) do
    table.insert(argv, "--assignee")
    table.insert(argv, tostring(assignee))
  end
  return argv
end

local function pr_create_argv(repo, branch, base_branch, title, body_file)
  local argv = { "gh", "pr", "create", "--repo", tostring(repo), "--head", tostring(branch) }
  if base_branch ~= nil then
    table.insert(argv, "--base")
    table.insert(argv, tostring(base_branch))
  end
  table.insert(argv, "--title")
  table.insert(argv, tostring(title))
  table.insert(argv, "--body-file")
  table.insert(argv, tostring(body_file))
  return argv
end

local function label_list_argv(repo)
  return { "gh", "label", "list", "--repo", tostring(repo), "--limit", "1000", "--json", "name" }
end

local function label_create_argv(repo, label, color)
  return { "gh", "label", "create", tostring(label), "--repo", tostring(repo), "--color", tostring(color or "ededed") }
end

local function edit_labels_argv(command, repo, number, add_labels, remove_labels)
  local argv = { "gh", command, "edit", tostring(number), "--repo", tostring(repo) }
  for _, label in ipairs(add_labels or {}) do
    table.insert(argv, "--add-label")
    table.insert(argv, tostring(label))
  end
  for _, label in ipairs(remove_labels or {}) do
    table.insert(argv, "--remove-label")
    table.insert(argv, tostring(label))
  end
  return argv
end

function M.install(handle)
  function handle.issue_list(repo, timeout)
    return handle._exec(issue_list_argv(repo), timeout, "gh issue list")
  end

  function handle.issue_list_cli(repo, state, limit, fields, timeout)
    return handle._exec(issue_list_cli_argv(repo, state, limit, fields), timeout, "gh issue list")
  end

  function handle.issue_list_recent_closed(repo, limit, timeout)
    local bounded_limit = tonumber(limit or 30)
    if bounded_limit == nil or bounded_limit < 1 or bounded_limit > 100 then
      error("std.github.entities: invalid closed issue list limit")
    end
    return handle.issue_list_cli(repo, "closed", math.floor(bounded_limit), "number,title,closedAt,labels", timeout)
  end

  function handle.issue_list_board_digest(repo, timeout)
    return handle.issue_list_cli(repo, "open", 100, "number,title,labels", timeout)
  end

  function handle.pr_list(repo, timeout)
    return handle._exec(pr_list_argv(repo), timeout, "gh pr list")
  end

  function handle.pr_list_cli(repo, state, limit, fields, timeout)
    return handle._exec(pr_list_cli_argv(repo, state, limit, fields), timeout, "gh pr list")
  end

  function handle.pr_list_board_digest(repo, timeout)
    return handle.pr_list_cli(repo, "open", 100, "number,title,labels", timeout)
  end

  function handle.pr_list_head(repo, branch, base_branch, timeout)
    return handle._exec(pr_list_head_argv(repo, branch, base_branch), timeout, "gh pr list --head")
  end

  function handle.pr_view(repo, pr_number, timeout)
    return handle._exec(pr_view_argv(repo, pr_number), timeout, "gh PR REST head repository/headRefOid/state")
  end

  function handle.pr_rest_view(repo, pr_number, timeout)
    return handle._exec(pr_view_argv(repo, pr_number), timeout, "gh PR REST view")
  end

  function handle.pr_updated_at(repo, pr_number, timeout)
    return handle._exec(entity_updated_at_argv(repo, "pr", pr_number), timeout, "gh PR updated_at")
  end

  function handle.issue_search(repo, query, fields, timeout)
    return handle._exec(issue_search_argv(repo, query, fields), timeout, "gh issue search")
  end

  function handle.issue_create(repo, title, body_file, labels, assignees, timeout)
    return handle._exec(issue_create_argv(repo, title, body_file, labels, assignees), timeout, "gh issue create")
  end

  function handle.pr_create(repo, branch, base_branch, title, body_file, timeout)
    return handle._exec(pr_create_argv(repo, branch, base_branch, title, body_file), timeout, "gh pr create")
  end

  function handle.label_list(repo, timeout)
    return handle._exec(label_list_argv(repo), timeout, "gh label list")
  end

  function handle.label_create(repo, label, color, timeout)
    return handle._exec(label_create_argv(repo, label, color), timeout, "gh label create")
  end

  function handle.issue_edit_labels(repo, issue_number, add_labels, remove_labels, timeout)
    return handle._exec(edit_labels_argv("issue", repo, issue_number, add_labels, remove_labels), timeout, "gh issue edit")
  end

  function handle.pr_edit_labels(repo, pr_number, add_labels, remove_labels, timeout)
    return handle._exec(edit_labels_argv("pr", repo, pr_number, add_labels, remove_labels), timeout, "gh pr edit")
  end
end

return M
