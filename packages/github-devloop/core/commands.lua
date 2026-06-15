local S = {}

function S.install(M)
local function url_encode(value)
  local text = tostring(value or "")
  return (text:gsub("([^%w%-%._~])", function(char)
    return string.format("%%%02X", string.byte(char))
  end))
end

local function repo_owner(repo)
  return tostring(repo or ""):match("^([^/]+)/")
end

function M.gh_issue_list_intake_cmd(repo, limit)
  local bounded_limit = tonumber(limit or 100)
  if bounded_limit == nil or bounded_limit < 1 or bounded_limit > 100 then
    error("github-devloop: invalid intake issue list limit")
  end
  return "gh issue list"
    .. " --repo " .. M._shell_single_quote(repo)
    .. " --state open"
    .. " --limit " .. tostring(math.floor(bounded_limit))
    .. " --json number,title,body,updatedAt,labels,assignees,author"
end

function M.gh_issue_list_intake_probe_cmd(repo, limit, since)
  local bounded_limit = tonumber(limit or 5)
  if bounded_limit == nil or bounded_limit < 1 or bounded_limit > 10 then
    error("github-devloop: invalid intake probe issue list limit")
  end
  local query = "repos/" .. tostring(repo)
    .. "/issues?state=open&sort=created&direction=desc&per_page=" .. tostring(math.floor(bounded_limit))
  if since ~= nil and tostring(since) ~= "" then
    query = query .. "&since=" .. url_encode(since)
  end
  return "gh api "
    .. M._shell_single_quote(query)
end

function M.gh_issue_list_decompose_children_cmd(repo, proposal_id)
  return "gh issue list"
    .. " --repo " .. M._shell_single_quote(repo)
    .. " --state all"
    .. " --limit 100"
    .. " --search " .. M._shell_single_quote("fkst:github-devloop:decompose-child:v1 " .. tostring(proposal_id))
    .. " --json number,title,state,author,body,url"
end

function M.gh_issue_list_recent_closed_cmd(repo, limit)
  local bounded_limit = tonumber(limit or 30)
  if bounded_limit == nil or bounded_limit < 1 or bounded_limit > 100 then
    error("github-devloop: invalid closed issue list limit")
  end
  return "gh issue list"
    .. " --repo " .. M._shell_single_quote(repo)
    .. " --state closed"
    .. " --limit " .. tostring(math.floor(bounded_limit))
    .. " --json number,title,closedAt,labels"
end

local function bounded_page_number(page)
  if page == nil then
    return nil
  end
  local n = tonumber(page)
  if n == nil or n ~= math.floor(n) or n < 1 then
    error("github-devloop: invalid list page number")
  end
  return n
end

local function observe_list_page_key(page)
  local selected_page = bounded_page_number(page)
  if selected_page == nil then
    return "paginate"
  end
  return tostring(selected_page)
end

local observe_list_timeout = 10

local function read_coalesce_key_segment(value, fallback)
  local text = tostring(value or "")
  if text == "" then
    return fallback or "all"
  end
  local segment = text:gsub("[^A-Za-z0-9%.%-]", function(char)
    return string.format("_%02X", string.byte(char))
  end)
  return "v-" .. segment
end

local function observe_list_repo_key(repo)
  local owner, name = tostring(repo or ""):match("^([^/]+)/([^/]+)$")
  if owner ~= nil and name ~= nil then
    return read_coalesce_key_segment(owner, "owner") .. "/" .. read_coalesce_key_segment(name, "repo")
  end
  return read_coalesce_key_segment(repo, "repo")
end

local function observe_list_label_key(label)
  if label == nil or tostring(label) == "" then
    return "all"
  end
  return read_coalesce_key_segment(label, "label")
end

local function observe_list_read_coalesce(key)
  return {
    key = key,
    ttl_seconds = 30,
  }
end

function M.gh_issue_list_observe_cmd(repo, label, page, include_headers)
  local selected_page = bounded_page_number(page)
  local include = include_headers and "--include " or ""
  local paginate = "--paginate --slurp "
  local page_query = ""
  if selected_page ~= nil then
    paginate = ""
    page_query = "&page=" .. tostring(selected_page)
  end
  if label == nil or tostring(label) == "" then
    return "gh api " .. include .. paginate
      .. M._shell_single_quote("repos/" .. tostring(repo) .. "/issues?state=open&per_page=100" .. page_query)
  end
  local selected_label = label
  return "gh api " .. include .. paginate
    .. M._shell_single_quote("repos/" .. tostring(repo) .. "/issues?state=open&labels=" .. tostring(selected_label):gsub(":", "%%3A") .. "&per_page=100" .. page_query)
end

function M.gh_issue_list_observe_read_coalesce(repo, label, page)
  return observe_list_read_coalesce(table.concat({
    "github-devloop",
    "observe-list",
    observe_list_repo_key(repo),
    "issues",
    "label",
    observe_list_label_key(label),
    "page",
    observe_list_page_key(page),
  }, "/"))
end

function M.gh_issue_list_observe_opts(repo, label, page, include_headers)
  return {
    cmd = M.gh_issue_list_observe_cmd(repo, label, page, include_headers),
    timeout = observe_list_timeout,
    read_coalesce = M.gh_issue_list_observe_read_coalesce(repo, label, page),
  }
end

function M.gh_issue_list_wip_cmd(repo)
  return "gh issue list"
    .. " --repo " .. M._shell_single_quote(repo)
    .. " --state open"
    .. " --label " .. M._shell_single_quote(M._enabled_label)
    .. " --limit 100"
    .. " --json number"
end

function M.gh_dashboard_issue_list_cmd(repo, label)
  local selected_label = tostring(label or "")
  if selected_label == "" then
    error("github-devloop: dashboard issue label is required")
  end
  return "gh api --paginate --slurp "
    .. M._shell_single_quote("repos/" .. tostring(repo) .. "/issues?state=open&labels=" .. selected_label:gsub(":", "%%3A") .. "&per_page=100")
end

function M.gh_dashboard_issue_all_open_cmd(repo)
  return "gh api --paginate --slurp "
    .. M._shell_single_quote("repos/" .. tostring(repo) .. "/issues?state=open&per_page=100")
end

function M.gh_dashboard_issue_add_label_cmd(repo, issue_number, label)
  local selected_label = tostring(label or "")
  if selected_label == "" then
    error("github-devloop: dashboard issue label is required")
  end
  return "gh api --method POST "
    .. M._shell_single_quote("repos/" .. tostring(repo) .. "/issues/" .. tostring(issue_number) .. "/labels")
    .. " -f " .. M._shell_single_quote("labels[]=" .. selected_label)
end

function M.gh_dashboard_label_get_cmd(repo, label)
  local selected_label = tostring(label or "")
  if selected_label == "" then
    error("github-devloop: dashboard label is required")
  end
  return "gh api --method GET "
    .. M._shell_single_quote("repos/" .. tostring(repo) .. "/labels/" .. selected_label:gsub(":", "%%3A"))
end

function M.gh_dashboard_label_create_cmd(repo, label)
  local selected_label = tostring(label or "")
  if selected_label == "" then
    error("github-devloop: dashboard label is required")
  end
  return "gh api --method POST "
    .. M._shell_single_quote("repos/" .. tostring(repo) .. "/labels")
    .. " -f " .. M._shell_single_quote("name=" .. selected_label)
    .. " -f " .. M._shell_single_quote("color=ededed")
    .. " -f " .. M._shell_single_quote("description=fkst observability dashboard singleton")
end

function M.gh_repo_labels_list_cmd(repo)
  return "gh api --paginate --slurp "
    .. M._shell_single_quote("repos/" .. tostring(repo) .. "/labels?per_page=100")
end

function M.gh_repo_label_create_cmd(repo, name, color, description)
  local selected_label = tostring(name or "")
  if selected_label == "" then
    error("github-devloop: label name is required")
  end
  local selected_color = tostring(color or "")
  if selected_color:find("^%x%x%x%x%x%x$") == nil then
    error("github-devloop: label color is invalid")
  end
  return "gh api --method POST "
    .. M._shell_single_quote("repos/" .. tostring(repo) .. "/labels")
    .. " -f " .. M._shell_single_quote("name=" .. selected_label)
    .. " -f " .. M._shell_single_quote("color=" .. selected_color)
    .. " -f " .. M._shell_single_quote("description=" .. tostring(description or ""))
end

function M.gh_repo_label_update_cmd(repo, name, color, description)
  local selected_label = tostring(name or "")
  if selected_label == "" then
    error("github-devloop: label name is required")
  end
  local selected_color = tostring(color or "")
  if selected_color:find("^%x%x%x%x%x%x$") == nil then
    error("github-devloop: label color is invalid")
  end
  return "gh api --method PATCH "
    .. M._shell_single_quote("repos/" .. tostring(repo) .. "/labels/" .. selected_label:gsub(":", "%%3A"))
    .. " -f " .. M._shell_single_quote("color=" .. selected_color)
    .. " -f " .. M._shell_single_quote("description=" .. tostring(description or ""))
end

function M.gh_dashboard_issue_create_cmd(repo, input_file)
  return "gh api --method POST "
    .. M._shell_single_quote("repos/" .. tostring(repo) .. "/issues")
    .. " --input " .. M._shell_single_quote(input_file)
end

function M.gh_dashboard_issue_get_cmd(repo, issue_number)
  return "gh api --method GET --include "
    .. M._shell_single_quote("repos/" .. tostring(repo) .. "/issues/" .. tostring(issue_number))
end

function M.gh_dashboard_issue_update_cmd(repo, issue_number, input_file)
  return "gh api --method PATCH "
    .. M._shell_single_quote("repos/" .. tostring(repo) .. "/issues/" .. tostring(issue_number))
    .. " --input " .. M._shell_single_quote(input_file)
end

function M.gh_pr_list_observe_cmd(repo, page, include_headers)
  local selected_page = bounded_page_number(page)
  local include = include_headers and "--include " or ""
  local paginate = "--paginate --slurp "
  local page_query = ""
  if selected_page ~= nil then
    paginate = ""
    page_query = "&page=" .. tostring(selected_page)
  end
  return "gh api " .. include .. paginate
    .. M._shell_single_quote("repos/" .. tostring(repo) .. "/pulls?state=open&per_page=100" .. page_query)
end

function M.gh_pr_list_observe_read_coalesce(repo, page)
  return observe_list_read_coalesce(table.concat({
    "github-devloop",
    "observe-list",
    observe_list_repo_key(repo),
    "prs",
    "page",
    observe_list_page_key(page),
  }, "/"))
end

function M.gh_pr_list_observe_opts(repo, page, include_headers)
  return {
    cmd = M.gh_pr_list_observe_cmd(repo, page, include_headers),
    timeout = observe_list_timeout,
    read_coalesce = M.gh_pr_list_observe_read_coalesce(repo, page),
  }
end

function M.gh_pr_list_freshness_cmd(repo)
  return "gh api --paginate --slurp "
    .. M._shell_single_quote("repos/" .. tostring(repo) .. "/pulls?state=open&per_page=100")
end

function M.gh_pr_list_merge_queue_cmd(repo, base)
  if not M._is_git_ref_safe(base) then
    error("github-devloop: invalid merge queue base branch")
  end
  return "gh api --paginate --slurp "
    .. M._shell_single_quote("repos/" .. tostring(repo)
      .. "/pulls?state=open&base=" .. url_encode(base)
      .. "&per_page=100")
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
  return M.gh_issue_view_cmd(repo, issue_number, "title,labels,comments,state,assignees,author")
end

function M.gh_issue_view_intake_judge_cmd(repo, issue_number)
  return M.gh_issue_view_cmd(repo, issue_number, "title,body,updatedAt,labels,comments,state,assignees,author")
end

function M.gh_issue_view_state_cmd(repo, issue_number)
  return M.gh_issue_view_cmd(repo, issue_number, "title,labels,state,comments,assignees,author")
end

function M.gh_issue_view_claim_cmd(repo, issue_number)
  return M.gh_issue_view_cmd(repo, issue_number, "assignees,author")
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
  return M.gh_issue_view_cmd(repo, issue_number, "title,labels,comments,assignees,author")
end

function M.gh_issue_view_reviewing_cmd(repo, issue_number)
  return M.gh_issue_view_cmd(repo, issue_number, "labels,comments")
end

function M.gh_issue_view_review_cmd(repo, issue_number)
  return M.gh_issue_view_cmd(repo, issue_number, "title,labels,comments,assignees,author")
end

function M.gh_issue_view_decompose_cmd(repo, issue_number)
  return M.gh_issue_view_cmd(repo, issue_number, "title,body,labels,comments")
end

function M.gh_issue_view_fix_cmd(repo, issue_number)
  return M.gh_issue_view_cmd(repo, issue_number, "title,labels,comments")
end

function M.gh_issue_view_commit_subject_cmd(repo, issue_number)
  return M.gh_issue_view_cmd(repo, issue_number, "number,title")
end

function M.gh_issue_view_review_loop_cmd(repo, issue_number)
  return M.gh_issue_view_cmd(repo, issue_number, "title,labels,comments,assignees,author")
end

function M.gh_issue_view_merge_cmd(repo, issue_number)
  return M.gh_issue_view_cmd(repo, issue_number, "title,labels,comments,state,assignees")
end

function M.gh_issue_view_observe_cmd(repo, issue_number)
  return M.gh_issue_view_cmd(repo, issue_number, "title,comments,state,stateReason,assignees,author")
end

function M.gh_pr_view_origin_cmd(repo, pr_number)
  return "gh pr view " .. M._shell_single_quote(pr_number)
    .. " --repo " .. M._shell_single_quote(repo)
    .. " --json headRefName,headRefOid,baseRefName,state,updatedAt,comments,labels,mergeable,mergeStateStatus"
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
    .. " --json headRefName,headRefOid,baseRefName,baseRefOid,state,updatedAt,isDraft,mergedAt,comments,headRepository,headRepositoryOwner,isCrossRepository,mergeable,mergeStateStatus,statusCheckRollup"
end

function M.gh_pr_view_freshness_cmd(repo, pr_number)
  return "gh pr view " .. M._shell_single_quote(pr_number)
    .. " --repo " .. M._shell_single_quote(repo)
    .. " --json headRefName,headRefOid,baseRefName,state,updatedAt,isDraft,comments,labels,headRepository,headRepositoryOwner,isCrossRepository,mergeable,mergeStateStatus,statusCheckRollup"
end

function M.gh_pr_list_head_base_cmd(repo, head, base)
  if not M._is_git_ref_safe(head) then
    error("github-devloop: invalid PR head branch")
  end
  if not M._is_git_ref_safe(base) then
    error("github-devloop: invalid PR base branch")
  end
  local owner = repo_owner(repo)
  local head_filter = owner ~= nil and (owner .. ":" .. tostring(head)) or tostring(head)
  return "gh api --paginate --slurp "
    .. M._shell_single_quote("repos/" .. tostring(repo)
      .. "/pulls?state=open&head=" .. url_encode(head_filter)
      .. "&base=" .. url_encode(base)
      .. "&per_page=100") -- gh api --paginate
end

function M.gh_pr_list_head_cmd(repo, head)
  if not M._is_git_ref_safe(head) then
    error("github-devloop: invalid PR head branch")
  end
  local owner = repo_owner(repo)
  local head_filter = owner ~= nil and (owner .. ":" .. tostring(head)) or tostring(head)
  return "gh api --paginate --slurp "
    .. M._shell_single_quote("repos/" .. tostring(repo)
      .. "/pulls?state=open&head=" .. url_encode(head_filter)
      .. "&per_page=100")
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

function M.gh_commit_check_runs_cmd(repo, head_sha)
  if not M._is_git_sha(head_sha) then
    error("github-devloop: invalid commit check-runs head sha")
  end
  return "gh api " .. M._shell_single_quote("repos/" .. tostring(repo) .. "/commits/" .. tostring(head_sha) .. "/check-runs")
end

function M.gh_issue_comment_get_cmd(repo, comment_id)
  if not M.is_safe_comment_id(comment_id) then
    error("github-devloop: invalid comment id")
  end
  return "gh api --method GET "
    .. M._shell_single_quote("repos/" .. tostring(repo) .. "/issues/comments/" .. tostring(comment_id))
end

function M.gh_pr_ready_cmd(repo, pr_number)
  return "gh pr ready " .. M._shell_single_quote(pr_number)
    .. " --repo " .. M._shell_single_quote(repo)
end

function M.gh_workflow_dispatch_ci_cmd(repo, ref)
  if not M._is_git_ref_safe(ref) then
    error("github-devloop: invalid workflow dispatch ref")
  end
  return "gh workflow run " .. M._shell_single_quote("ci.yml")
    .. " --repo " .. M._shell_single_quote(repo)
    .. " --ref " .. M._shell_single_quote(ref)
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

function M.gh_pr_close_cmd(repo, pr_number)
  return "gh pr close " .. M._shell_single_quote(pr_number)
    .. " --repo " .. M._shell_single_quote(repo)
end

function M.gh_issue_close_cmd(repo, issue_number)
  return "gh issue close " .. M._shell_single_quote(issue_number)
    .. " --repo " .. M._shell_single_quote(repo)
end

function M.gh_pr_diff_cmd(repo, pr_number)
  return "gh pr diff " .. M._shell_single_quote(pr_number)
    .. " --repo " .. M._shell_single_quote(repo)
end

function M.gh_pr_diff_name_only_cmd(repo, pr_number)
  return "gh pr diff " .. M._shell_single_quote(pr_number)
    .. " --repo " .. M._shell_single_quote(repo)
    .. " --name-only"
end

function M.gh_pr_view_head_cmd(repo, pr_number)
  return "gh pr view " .. M._shell_single_quote(pr_number)
    .. " --repo " .. M._shell_single_quote(repo)
    .. " --json headRefName,baseRefName,state"
end

function M.gh_pr_view_context_cmd(repo, pr_number)
  return "gh pr view " .. M._shell_single_quote(pr_number)
    .. " --repo " .. M._shell_single_quote(repo)
    .. " --json title,body,headRefName,headRefOid,baseRefName,state,updatedAt,comments,labels"
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

function M.git_ls_remote_branch_cmd(remote, branch)
  local selected_remote = tostring(remote or "")
  if selected_remote == "" or selected_remote:find("[\r\n]") ~= nil then
    error("github-devloop: invalid git remote")
  end
  if not M._is_git_ref_safe(branch) then
    error("github-devloop: invalid remote branch")
  end
  return "git ls-remote " .. M._shell_single_quote(selected_remote)
    .. " " .. M._shell_single_quote("refs/heads/" .. tostring(branch))
end

function M.git_fetch_pr_merge_ref_cmd(remote, pr_number)
  if not M._is_git_ref_safe(remote) then
    error("github-devloop: invalid git remote")
  end
  if not M._is_positive_pr_number(pr_number) then
    error("github-devloop: invalid pull request number")
  end
  return "git fetch " .. M._shell_single_quote(remote) .. " "
    .. M._shell_single_quote("refs/pull/" .. tostring(pr_number) .. "/merge")
end

function M.git_fetch_pr_head_ref_cmd(remote, pr_number)
  if not M._is_git_ref_safe(remote) then
    error("github-devloop: invalid git remote")
  end
  if not M._is_positive_pr_number(pr_number) then
    error("github-devloop: invalid pull request number")
  end
  return "git fetch " .. M._shell_single_quote(remote) .. " "
    .. M._shell_single_quote("refs/pull/" .. tostring(pr_number) .. "/head")
end

function M.git_fetch_head_commit_cmd()
  return "git rev-parse --verify FETCH_HEAD^{commit}"
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

function M.git_worktree_merge_no_edit_cmd(worktree, sha)
  if not M._is_git_sha(sha) then
    error("github-devloop: invalid merge sha")
  end
  return "git -C " .. M._shell_single_quote(worktree)
    .. " merge --no-edit "
    .. M._shell_single_quote(sha)
end

function M.git_worktree_reset_hard_cmd(worktree, branch)
  if not M._is_git_ref_safe(branch) then
    error("github-devloop: invalid reset branch")
  end
  return "git -C " .. M._shell_single_quote(worktree)
    .. " reset --hard refs/heads/"
    .. M._shell_single_quote(branch)
end

function M.git_worktree_clean_cmd(worktree)
  return "git -C " .. M._shell_single_quote(worktree) .. " clean -fd"
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

function M.git_switch_branch_cmd(worktree, branch)
  if not M._is_git_ref_safe(branch) then
    error("github-devloop: invalid branch")
  end
  return "git -C " .. M._shell_single_quote(worktree)
    .. " switch " .. M._shell_single_quote(branch)
end

function M.git_write_file_cmd(worktree, relative_path, contents)
  local path = tostring(relative_path or "")
  if path == "" or path:find("[\r\n]") ~= nil or path:sub(1, 1) == "/" or path:find("%.%.", 1, true) ~= nil then
    error("github-devloop: invalid write path")
  end
  return "printf %s " .. M._shell_single_quote(tostring(contents or ""))
    .. " > " .. M._shell_single_quote(tostring(worktree):gsub("/+$", "") .. "/" .. path)
end

function M.read_runtime_root_cmd()
  return 'printf %s "$FKST_RUNTIME_ROOT"'
end

function M.mkdir_p_cmd(path)
  local value = tostring(path or "")
  if value == "" or value:find("[\r\n]") ~= nil then
    error("github-devloop: invalid directory path")
  end
  return "mkdir -p " .. M._shell_single_quote(value) .. " && chmod 0555 " .. M._shell_single_quote(value)
end

function M.git_worktree_remove_if_present_cmd(worktree)
  local value = tostring(worktree or "")
  if value == "" or value:find("[\r\n]") ~= nil then
    error("github-devloop: invalid worktree path")
  end
  return "if [ -d " .. M._shell_single_quote(value) .. " ]; then git worktree remove --force " .. M._shell_single_quote(value) .. "; fi"
end

function M.git_worktree_force_clean_cmd(worktree)
  local value = tostring(worktree or "")
  if value == "" or value:find("[\r\n]") ~= nil then
    error("github-devloop: invalid worktree path")
  end
  local w = M._shell_single_quote(value)
  -- Idempotently clear the target worktree path before `git worktree add`. Robust
  -- to an orphan dir (present on disk but with no registration, left by a codex
  -- killed mid-implement on restart) as well as a registered worktree. Always
  -- exits 0 so a missing path is a clean no-op. The target is clearable scratch
  -- under FKST_RUNTIME_ROOT.
  return "git worktree remove --force " .. w .. " 2>/dev/null; rm -rf " .. w .. "; git worktree prune"
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

function M.git_worktree_add_reset_branch_cmd(worktree, branch, base)
  if not M._is_git_ref_safe(branch) then
    error("github-devloop: invalid branch")
  end
  if not M._is_git_sha(base) then
    error("github-devloop: invalid base head")
  end
  return "mkdir -p " .. M._shell_single_quote(tostring(worktree):gsub("/+$", ""):match("^(.*)/[^/]+$") or ".")
    .. " && git worktree add -B " .. M._shell_single_quote(branch)
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

function M.git_worktree_add_remote_branch_cmd(worktree, remote, branch, force)
  if not M._is_git_ref_safe(remote) then
    error("github-devloop: invalid git remote")
  end
  if not M._is_git_ref_safe(branch) then
    error("github-devloop: invalid branch")
  end
  local force_arg = ""
  if force then
    force_arg = " --force"
  end
  return "mkdir -p " .. M._shell_single_quote(tostring(worktree):gsub("/+$", ""):match("^(.*)/[^/]+$") or ".")
    .. " && git worktree add" .. force_arg
    .. " -B " .. M._shell_single_quote(branch)
    .. " " .. M._shell_single_quote(worktree)
    .. " refs/remotes/" .. M._shell_single_quote(remote) .. "/" .. M._shell_single_quote(branch)
end

function M.git_worktree_list_cmd()
  return "git worktree list --porcelain"
end

function M.git_worktree_prune_cmd()
  return "git worktree prune"
end

function M.path_is_directory_cmd(path)
  local value = tostring(path or "")
  if value == "" or value:find("[\r\n]") ~= nil then
    error("github-devloop: invalid directory path")
  end
  return "[ -d " .. M._shell_single_quote(value) .. " ]"
end

function M.find_worktrees_for_branch(stdout, branch)
  if not M._is_git_ref_safe(branch) then
    error("github-devloop: invalid branch")
  end
  local wanted = "refs/heads/" .. tostring(branch)
  local path = nil
  local matches = {}
  for line in (tostring(stdout or "") .. "\n"):gmatch("([^\n]*)\n") do
    if line == "" then
      path = nil
    else
      local current_path = line:match("^worktree%s+(.+)$")
      if current_path ~= nil then
        path = current_path
      elseif line == "branch " .. wanted and path ~= nil and path ~= "" then
        table.insert(matches, path)
      end
    end
  end
  return matches
end

function M.find_worktree_for_branch(stdout, branch)
  local matches = M.find_worktrees_for_branch(stdout, branch)
  if #matches > 0 then
    return matches[1]
  end
  return nil
end

function M.find_worktree_for_branch_under_runtime(stdout, branch, runtime_root)
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
      elseif line == "branch " .. wanted
        and path ~= nil
        and path ~= ""
        and M.path_under_runtime_root(runtime_root, path) then
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
