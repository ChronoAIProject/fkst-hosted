local S = {}

function S.install(M)
local function fetch_event_view(cmd, cache_key, opts)
  local options = opts or {}
  if options.fresh == true or cache_key == nil or tostring(cache_key) == "" then
    return exec_sync({ cmd = cmd, timeout = 30 })
  end

  local cached = cache_get(cache_key)
  if cached ~= nil and cached ~= "" then
    return {
      stdout = cached,
      stderr = "",
      exit_code = 0,
    }
  end

  local result = exec_sync({ cmd = cmd, timeout = 30 })
  if result.exit_code == 0 then
    cache_set(cache_key, result.stdout or "")
  end
  return result
end

function M.gh_issue_view_entity_cmd(repo, issue_number)
  return M.gh_issue_view_cmd(repo, issue_number, "title,body,comments,labels,state")
end

function M.gh_pr_view_entity_cmd(repo, pr_number)
  return M.gh_pr_view_origin_cmd(repo, pr_number)
end

function M.fetch_issue_view_state(repo, issue_number, opts)
  return fetch_event_view(M.gh_issue_view_entity_cmd(repo, issue_number), opts and opts.cache_key, opts)
end

function M.fetch_issue_view_open_pr(repo, issue_number, opts)
  return fetch_event_view(M.gh_issue_view_entity_cmd(repo, issue_number), opts and opts.cache_key, opts)
end

function M.fetch_pr_view_origin(repo, pr_number, opts)
  return fetch_event_view(M.gh_pr_view_entity_cmd(repo, pr_number), opts and opts.cache_key, opts)
end

end

return S
