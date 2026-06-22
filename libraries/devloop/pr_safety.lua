local S = {}

function S.install(M)
function M.is_safe_branch(branch)
  return M._is_git_ref_safe(branch)
end

function M.is_devloop_issue_branch(branch)
  return type(branch) == "string"
    and M._is_git_ref_safe(branch)
    and branch:find("^devloop/issue/[^/]+/.+/.+") ~= nil
end

function M.is_safe_head_sha(head_sha)
  return M._is_git_sha(head_sha)
end

function M.is_safe_pr_number(pr_number)
  return M._is_positive_pr_number(pr_number)
end

function M.is_same_repo_pr_head(pr, repo)
  if type(pr) ~= "table" then
    return false
  end
  if pr.is_cross_repository == true then
    return false
  end
  if pr.head_repository == nil then
    return false
  end
  return tostring(pr.head_repository):lower() == tostring(repo):lower()
end
end

return S
