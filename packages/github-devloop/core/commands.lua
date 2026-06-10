local S = {}

function S.install(M)
function M.gh_issue_list_intake_cmd(repo, limit)
  local bounded_limit = tonumber(limit or 100)
  if bounded_limit == nil or bounded_limit < 1 or bounded_limit > 100 then
    error("github-devloop: invalid intake issue list limit")
  end
  return "gh issue list"
    .. " --repo " .. M._shell_single_quote(repo)
    .. " --state open"
    .. " --limit " .. tostring(math.floor(bounded_limit))
    .. " --json number,title,updatedAt,labels"
end

function M.gh_issue_list_observe_cmd(repo, label)
  local selected_label = label or M._enabled_label
  return "gh api --paginate --slurp "
    .. M._shell_single_quote("repos/" .. tostring(repo) .. "/issues?state=open&labels=" .. tostring(selected_label):gsub(":", "%%3A") .. "&per_page=100")
end

function M.gh_pr_list_observe_cmd(repo)
  return "gh api --paginate --slurp "
    .. M._shell_single_quote("repos/" .. tostring(repo) .. "/pulls?state=open&per_page=100")
end

function M.gh_issue_view_cmd(repo, issue_number, fields)
  local selected_fields = tostring(fields or "")
  if selected_fields == "" or selected_fields:match("[^%w_,]") or selected_fields:match("^,") or selected_fields:match(",$") or selected_fields:match(",,") then
    error("github-devloop: invalid issue view fields")
  end
  return "gh issue view " .. M._shell_single_quote(issue_number)
    .. " --repo " .. M._shell_single_quote(repo)
    .. " --json " .. selected_fields
end

function M.gh_issue_view_intake_scan_cmd(repo, issue_number)
  return M.gh_issue_view_cmd(repo, issue_number, "labels,comments,state")
end

function M.gh_issue_view_intake_judge_cmd(repo, issue_number)
  return M.gh_issue_view_cmd(repo, issue_number, "title,body,updatedAt,labels,comments,state")
end

function M.gh_issue_view_state_cmd(repo, issue_number)
  return M.gh_issue_view_cmd(repo, issue_number, "labels,state,comments")
end

function M.gh_issue_view_result_cmd(repo, issue_number)
  return M.gh_issue_view_cmd(repo, issue_number, "labels,comments")
end

function M.gh_issue_view_loop_cmd(repo, issue_number)
  return M.gh_issue_view_cmd(repo, issue_number, "title,updatedAt,labels,comments,state")
end

function M.gh_issue_view_meta_cmd(repo, issue_number)
  return M.gh_issue_view_cmd(repo, issue_number, "title,labels,comments")
end

function M.gh_issue_view_implement_cmd(repo, issue_number)
  return M.gh_issue_view_cmd(repo, issue_number, "title,labels,comments")
end

function M.gh_issue_view_open_pr_cmd(repo, issue_number)
  return M.gh_issue_view_cmd(repo, issue_number, "title,labels,comments")
end

function M.gh_issue_view_reviewing_cmd(repo, issue_number)
  return M.gh_issue_view_cmd(repo, issue_number, "labels,comments")
end

function M.gh_issue_view_review_cmd(repo, issue_number)
  return M.gh_issue_view_cmd(repo, issue_number, "title,labels,comments")
end

function M.gh_issue_view_decompose_cmd(repo, issue_number)
  return M.gh_issue_view_cmd(repo, issue_number, "title,body,labels,comments")
end

function M.gh_issue_view_fix_cmd(repo, issue_number)
  return M.gh_issue_view_cmd(repo, issue_number, "title,labels,comments")
end

function M.gh_issue_view_review_loop_cmd(repo, issue_number)
  return M.gh_issue_view_cmd(repo, issue_number, "title,labels,comments")
end

function M.gh_issue_view_merge_cmd(repo, issue_number)
  return M.gh_issue_view_cmd(repo, issue_number, "title,labels,comments,state")
end

function M.gh_issue_view_observe_cmd(repo, issue_number)
  return M.gh_issue_view_cmd(repo, issue_number, "comments,state")
end

function M.gh_pr_view_origin_cmd(repo, pr_number)
  return "gh pr view " .. M._shell_single_quote(pr_number)
    .. " --repo " .. M._shell_single_quote(repo)
    .. " --json headRefName,headRefOid,baseRefName,state,updatedAt,comments"
end

function M.gh_pr_view_observe_cmd(repo, pr_number)
  return M.gh_pr_view_origin_cmd(repo, pr_number)
end

function M.gh_pr_view_fix_cmd(repo, pr_number)
  return "gh pr view " .. M._shell_single_quote(pr_number)
    .. " --repo " .. M._shell_single_quote(repo)
    .. " --json headRefName,headRefOid,baseRefName,state,comments,headRepository,headRepositoryOwner,isCrossRepository"
end

function M.gh_pr_view_merge_cmd(repo, pr_number)
  return "gh pr view " .. M._shell_single_quote(pr_number)
    .. " --repo " .. M._shell_single_quote(repo)
    .. " --json headRefName,headRefOid,baseRefName,state,mergedAt,comments,headRepository,headRepositoryOwner,isCrossRepository,mergeable,mergeStateStatus,statusCheckRollup"
end

function M.gh_pr_list_head_base_cmd(repo, head, base)
  if not M._is_git_ref_safe(head) then
    error("github-devloop: invalid PR head branch")
  end
  if not M._is_git_ref_safe(base) then
    error("github-devloop: invalid PR base branch")
  end
  return "gh pr list"
    .. " --repo " .. M._shell_single_quote(repo)
    .. " --head " .. M._shell_single_quote(head)
    .. " --base " .. M._shell_single_quote(base)
    .. " --state open"
    .. " --json number,headRefOid,headRefName,baseRefName,state"
end

function M.gh_pr_create_cmd(repo, head, base, title, body_file)
  if not M._is_git_ref_safe(head) then
    error("github-devloop: invalid PR head branch")
  end
  if not M._is_git_ref_safe(base) then
    error("github-devloop: invalid PR base branch")
  end
  return "gh pr create --repo " .. M._shell_single_quote(repo)
    .. " --head " .. M._shell_single_quote(head)
    .. " --base " .. M._shell_single_quote(base)
    .. " --title " .. M._shell_single_quote(title)
    .. " --body-file " .. M._shell_single_quote(body_file)
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

function M.gh_pr_comment_cmd(repo, pr_number, body_file)
  return "gh pr comment " .. M._shell_single_quote(pr_number)
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
    .. " --json headRefName,baseRefName,state"
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

function M.git_base_head_cmd(branch)
  if not M._is_git_ref_safe(branch) then
    error("github-devloop: invalid base branch")
  end
  return "git rev-parse --verify refs/remotes/origin/" .. M._shell_single_quote(branch) .. "^{commit}"
end

function M.git_fetch_branch_cmd(remote, branch)
  if not M._is_git_ref_safe(remote) then
    error("github-devloop: invalid git remote")
  end
  if not M._is_git_ref_safe(branch) then
    error("github-devloop: invalid fetch branch")
  end
  return "git fetch " .. M._shell_single_quote(remote) .. " " .. M._shell_single_quote(branch)
end

function M.git_remote_branch_head_cmd(remote, branch)
  if not M._is_git_ref_safe(remote) then
    error("github-devloop: invalid git remote")
  end
  if not M._is_git_ref_safe(branch) then
    error("github-devloop: invalid remote branch")
  end
  return "git rev-parse --verify refs/remotes/" .. M._shell_single_quote(remote) .. "/" .. M._shell_single_quote(branch) .. "^{commit}"
end

function M.git_ahead_count_cmd(upstream, integration)
  if not M._is_git_ref_safe(upstream) then
    error("github-devloop: invalid upstream branch")
  end
  if not M._is_git_ref_safe(integration) then
    error("github-devloop: invalid integration branch")
  end
  return "git rev-list --count refs/remotes/origin/" .. M._shell_single_quote(upstream)
    .. "..refs/remotes/origin/" .. M._shell_single_quote(integration)
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
