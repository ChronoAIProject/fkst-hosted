local M = {}
local issue = require("std.github.issue")

local function copy(value)
  if type(value) ~= "table" then
    return value
  end
  local result = {}
  for key, field in pairs(value) do
    result[copy(key)] = copy(field)
  end
  return result
end

function M.model(seed)
  return {
    issues = seed and seed.issues or {},
    writes = seed and seed.writes or {},
  }
end

function M.new(model)
  assert(type(model) == "table", "std.github_fake.new requires a model")
  local handle = { _model = model }
  function handle._exec(argv, timeout, context)
    table.insert(model.writes, {
      kind = "exec",
      argv = copy(argv),
      timeout = timeout,
      context = context,
    })
    return { stdout = "", stderr = "", exit_code = 0 }
  end
  function handle.read_issue(source_ref)
    local fixture = model.issues[source_ref.ref]
    if fixture == nil then
      error("fake: unknown issue " .. tostring(source_ref.ref))
    end
    return copy(issue.normalize_issue(fixture, source_ref))
  end
  require("std.github.entities").install(handle)
  require("std.github.comments").install(handle)
  function handle.issue_rest_view(repo, issue_number, timeout)
    return handle._exec({ "gh", "api", "repos/" .. tostring(repo) .. "/issues/" .. tostring(issue_number) }, timeout, "gh issue REST view")
  end
  function handle.issue_updated_at(repo, issue_number, timeout)
    return handle._exec({
      "gh",
      "api",
      "repos/" .. tostring(repo) .. "/issues/" .. tostring(issue_number),
      "--jq",
      ".updated_at // .updatedAt // \"\"",
    }, timeout, "gh issue updated_at")
  end
  function handle.entity_updated_at(repo, kind, number, timeout)
    if kind == "pr" then
      return handle.pr_updated_at(repo, number, timeout)
    end
    return handle.issue_updated_at(repo, number, timeout)
  end
  function handle.issue_assign(repo, issue_number, login, timeout)
    return handle._exec({
      "gh",
      "issue",
      "edit",
      tostring(issue_number),
      "--repo",
      tostring(repo),
      "--add-assignee",
      tostring(login),
    }, timeout, "gh issue assign")
  end
  function handle.issue_unassign(repo, issue_number, login, timeout)
    return handle._exec({
      "gh",
      "issue",
      "edit",
      tostring(issue_number),
      "--repo",
      tostring(repo),
      "--remove-assignee",
      tostring(login),
    }, timeout, "gh issue unassign")
  end
  function handle.graphql(query, fields, timeout)
    local argv = { "gh", "api", "graphql", "-f", "query=" .. tostring(query) }
    for key, value in pairs(fields or {}) do
      table.insert(argv, "-f")
      table.insert(argv, tostring(key) .. "=" .. tostring(value))
    end
    return handle._exec(argv, timeout, "gh GraphQL")
  end
  return handle
end

return M
