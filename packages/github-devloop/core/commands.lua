local S = {}

function S.install(M)
function M.gh_issue_view_body_cmd(repo, issue_number)
  return "gh issue view " .. M._shell_single_quote(issue_number)
    .. " --repo " .. M._shell_single_quote(repo)
    .. " --json body"
end

function M.gh_issue_view_state_cmd(repo, issue_number)
  return "gh issue view " .. M._shell_single_quote(issue_number)
    .. " --repo " .. M._shell_single_quote(repo)
    .. " --json labels,state,comments"
end

function M.gh_issue_view_result_cmd(repo, issue_number)
  return "gh issue view " .. M._shell_single_quote(issue_number)
    .. " --repo " .. M._shell_single_quote(repo)
    .. " --json labels,comments"
end

function M.gh_issue_view_loop_cmd(repo, issue_number)
  return "gh issue view " .. M._shell_single_quote(issue_number)
    .. " --repo " .. M._shell_single_quote(repo)
    .. " --json title,body,updatedAt,labels,comments,state"
end

function M.gh_issue_view_meta_cmd(repo, issue_number)
  return "gh issue view " .. M._shell_single_quote(issue_number)
    .. " --repo " .. M._shell_single_quote(repo)
    .. " --json title,body,labels,comments"
end

function M.gh_issue_view_implement_cmd(repo, issue_number)
  return "gh issue view " .. M._shell_single_quote(issue_number)
    .. " --repo " .. M._shell_single_quote(repo)
    .. " --json title,body,labels,comments"
end

function M.gh_issue_view_open_pr_cmd(repo, issue_number)
  return "gh issue view " .. M._shell_single_quote(issue_number)
    .. " --repo " .. M._shell_single_quote(repo)
    .. " --json title,labels,comments"
end

function M.gh_issue_view_reviewing_cmd(repo, issue_number)
  return "gh issue view " .. M._shell_single_quote(issue_number)
    .. " --repo " .. M._shell_single_quote(repo)
    .. " --json labels,comments"
end

function M.gh_issue_view_review_cmd(repo, issue_number)
  return "gh issue view " .. M._shell_single_quote(issue_number)
    .. " --repo " .. M._shell_single_quote(repo)
    .. " --json title,body,labels,comments"
end

function M.gh_issue_view_fix_cmd(repo, issue_number)
  return "gh issue view " .. M._shell_single_quote(issue_number)
    .. " --repo " .. M._shell_single_quote(repo)
    .. " --json title,body,labels,comments"
end

function M.gh_issue_view_review_loop_cmd(repo, issue_number)
  return "gh issue view " .. M._shell_single_quote(issue_number)
    .. " --repo " .. M._shell_single_quote(repo)
    .. " --json title,body,labels,comments"
end

function M.gh_issue_view_review_meta_cmd(repo, issue_number)
  return "gh issue view " .. M._shell_single_quote(issue_number)
    .. " --repo " .. M._shell_single_quote(repo)
    .. " --json title,body,labels,comments"
end

function M.gh_issue_view_merge_cmd(repo, issue_number)
  return "gh issue view " .. M._shell_single_quote(issue_number)
    .. " --repo " .. M._shell_single_quote(repo)
    .. " --json title,body,labels,comments,state"
end

function M.gh_pr_view_origin_cmd(repo, pr_number)
  return "gh pr view " .. M._shell_single_quote(pr_number)
    .. " --repo " .. M._shell_single_quote(repo)
    .. " --json headRefName,headRefOid,state,comments"
end

function M.gh_pr_view_fix_cmd(repo, pr_number)
  return "gh pr view " .. M._shell_single_quote(pr_number)
    .. " --repo " .. M._shell_single_quote(repo)
    .. " --json headRefName,headRefOid,state,comments,headRepository,headRepositoryOwner,isCrossRepository"
end

function M.gh_pr_view_merge_cmd(repo, pr_number)
  return "gh pr view " .. M._shell_single_quote(pr_number)
    .. " --repo " .. M._shell_single_quote(repo)
    .. " --json headRefName,headRefOid,state,mergedAt,comments,headRepository,headRepositoryOwner,isCrossRepository,mergeable,mergeStateStatus,statusCheckRollup"
end

function M.gh_pr_merge_cmd(repo, pr_number, head_sha)
  if tostring(head_sha or "") == "" then
    error("github-devloop: invalid merge head sha")
  end
  return "gh pr merge " .. M._shell_single_quote(pr_number)
    .. " --repo " .. M._shell_single_quote(repo)
    .. " --merge"
    .. " --match-head-commit " .. M._shell_single_quote(head_sha)
end

function M.gh_issue_comment_cmd(repo, issue_number, body_file)
  return "gh issue comment " .. M._shell_single_quote(issue_number)
    .. " --repo " .. M._shell_single_quote(repo)
    .. " --body-file " .. M._shell_single_quote(body_file)
end

function M.gh_issue_close_cmd(repo, issue_number)
  return "gh issue close " .. M._shell_single_quote(issue_number)
    .. " --repo " .. M._shell_single_quote(repo)
end

function M.gh_pr_diff_cmd(repo, pr_number)
  return "gh pr diff " .. M._shell_single_quote(pr_number)
    .. " --repo " .. M._shell_single_quote(repo)
end

function M.gh_pr_view_head_cmd(repo, pr_number)
  return "gh pr view " .. M._shell_single_quote(pr_number)
    .. " --repo " .. M._shell_single_quote(repo)
    .. " --json headRefName,state"
end

function M.git_status_cmd(worktree)
  return "git -C " .. M._shell_single_quote(worktree) .. " status --porcelain"
end

function M.git_add_all_cmd(worktree)
  return "git -C " .. M._shell_single_quote(worktree) .. " add -A"
end

function M.git_commit_cmd(worktree, message)
  local bounded_message = tostring(message or "")
  if bounded_message == "" or #bounded_message > 200 then
    error("github-devloop: invalid git commit message")
  end
  return "git -C " .. M._shell_single_quote(worktree) .. " commit -m " .. M._shell_single_quote(bounded_message)
end

function M.git_current_branch_cmd(worktree)
  return "git -C " .. M._shell_single_quote(worktree) .. " rev-parse --abbrev-ref HEAD"
end

function M.git_head_sha_cmd(worktree)
  return "git -C " .. M._shell_single_quote(worktree) .. " rev-parse HEAD"
end

function M.git_base_head_cmd()
  return "git rev-parse HEAD"
end

function M.git_show_ref_branch_cmd(branch)
  return "git show-ref --verify --quiet refs/heads/" .. M._shell_single_quote(branch)
end

function M.git_show_ref_cmd(worktree, branch)
  return "git -C " .. M._shell_single_quote(worktree) .. " show-ref --verify --quiet refs/heads/" .. M._shell_single_quote(branch)
end

function M.git_branch_ahead_count_cmd(base, branch)
  if not M._is_git_sha(base) then
    error("github-devloop: invalid base head")
  end
  if not M._is_git_ref_safe(branch) then
    error("github-devloop: invalid branch")
  end
  return "git rev-list --count " .. M._shell_single_quote(base) .. "..refs/heads/" .. M._shell_single_quote(branch)
end

function M.git_branch_head_cmd(branch)
  if not M._is_git_ref_safe(branch) then
    error("github-devloop: invalid branch")
  end
  return "git rev-parse --verify refs/heads/" .. M._shell_single_quote(branch)
end

function M.git_push_branch_cmd(branch)
  if not M._is_git_ref_safe(branch) then
    error("github-devloop: invalid branch")
  end
  return "git push origin " .. M._shell_single_quote(branch)
end

function M.read_runtime_root_cmd()
  return 'printf %s "$FKST_RUNTIME_ROOT"'
end

function M.git_worktree_add_new_branch_cmd(worktree, branch, base)
  if not M._is_git_ref_safe(branch) then
    error("github-devloop: invalid branch")
  end
  if not M._is_git_sha(base) then
    error("github-devloop: invalid base head")
  end
  return "mkdir -p " .. M._shell_single_quote(tostring(worktree):gsub("/+$", ""):match("^(.*)/[^/]+$") or ".")
    .. " && git worktree add -b " .. M._shell_single_quote(branch)
    .. " " .. M._shell_single_quote(worktree)
    .. " " .. M._shell_single_quote(base)
end

function M.git_worktree_add_existing_branch_cmd(worktree, branch)
  if not M._is_git_ref_safe(branch) then
    error("github-devloop: invalid branch")
  end
  return "mkdir -p " .. M._shell_single_quote(tostring(worktree):gsub("/+$", ""):match("^(.*)/[^/]+$") or ".")
    .. " && git worktree add " .. M._shell_single_quote(worktree)
    .. " " .. M._shell_single_quote(branch)
end

function M.git_worktree_list_cmd()
  return "git worktree list --porcelain"
end

function M.find_worktree_for_branch(stdout, branch)
  if not M._is_git_ref_safe(branch) then
    error("github-devloop: invalid branch")
  end
  local wanted = "refs/heads/" .. tostring(branch)
  local path = nil
  for line in (tostring(stdout or "") .. "\n"):gmatch("([^\n]*)\n") do
    if line == "" then
      path = nil
    else
      local current_path = line:match("^worktree%s+(.+)$")
      if current_path ~= nil then
        path = current_path
      elseif line == "branch " .. wanted and path ~= nil and path ~= "" then
        return path
      end
    end
  end
  return nil
end

function M.git_rev_parse_branch_cmd(worktree, branch)
  return "git -C " .. M._shell_single_quote(worktree) .. " rev-parse --verify refs/heads/" .. M._shell_single_quote(branch)
end
end

return S
