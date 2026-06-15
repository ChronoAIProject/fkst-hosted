local S = {}

local command_templates = {
  graphql_query = "gh api graphql -f query=",
}

local queries = {
  dependency_blocked_by = '{repository(owner:"{{owner}}",name:"{{name}}"){issue(number:{{issue_number}}){blockedBy(first:50){totalCount pageInfo{hasNextPage} nodes{number state stateReason repository{nameWithOwner}}}}}}',
}

local function render_query(template, fields)
  return tostring(template or ""):gsub("{{([%w_]+)}}", function(name)
    local value = fields and fields[name]
    if value == nil then
      error("github-devloop: graphql-template-missing-field: " .. tostring(name))
    end
    return tostring(value)
  end)
end

function S.install(M)
  M.github_graphql_command_templates = command_templates
  M.github_graphql_queries = queries

  function M.render_github_graphql_query(name, fields)
    local template = queries[name]
    if template == nil then
      error("github-devloop: graphql-template-unknown-query: " .. tostring(name))
    end
    return render_query(template, fields)
  end
end

return S
