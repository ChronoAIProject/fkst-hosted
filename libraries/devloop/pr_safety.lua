local C = {}
local forge_validators = require("devloop.forge_validators")

function C.is_safe_branch(M, branch)
  return forge_validators.is_git_ref_safe(branch)
end

function C.is_devloop_issue_branch(M, branch)
  return type(branch) == "string"
    and forge_validators.is_git_ref_safe(branch)
    and branch:find("^devloop/issue/[^/]+/.+/.+") ~= nil
end

function C.is_safe_head_sha(M, head_sha)
  return forge_validators.is_git_sha(head_sha)
end

function C.is_safe_pr_number(M, pr_number)
  return M._is_positive_pr_number(pr_number)
end

function C.is_same_repo_pr_head(M, pr, repo)
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

return C
