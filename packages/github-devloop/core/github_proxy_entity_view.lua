local S = {}

function S.install(M)
local installer = dofile("packages/github-proxy/core/entity_view.lua")

installer.install(M)

function M.fetch_issue_view_state(repo, issue_number, updated_at, opts)
  return M.fetch_issue_view(repo, issue_number, updated_at, opts)
end

function M.fetch_issue_view_open_pr(repo, issue_number, updated_at, opts)
  return M.fetch_issue_view(repo, issue_number, updated_at, opts)
end

function M.fetch_pr_view_origin(repo, pr_number, updated_at, opts)
  return M.fetch_pr_view(repo, pr_number, updated_at, opts)
end

end

return S
