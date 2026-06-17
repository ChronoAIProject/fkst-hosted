local S = {}

function S.install(M)
local github_handle = nil
local git_handle = nil

local function github(run)
  if run ~= nil then
    if type(run) ~= "function" then
      error("github-devloop: GitHub adapter requires an exec function")
    end
    return require("std.github").new(run)
  end
  if github_handle == nil then
    if type(exec_argv) ~= "function" then
      error("github-devloop: GitHub adapter requires exec_argv")
    end
    github_handle = require("std.github").new(exec_argv)
  end
  return github_handle
end

local function gh_result(fn)
  local ok, result_or_error = pcall(fn)
  if ok then
    return result_or_error
  end
  if type(result_or_error) == "table" and result_or_error.result ~= nil then
    return result_or_error.result
  end
  error(result_or_error)
end

local function git()
  if git_handle == nil then
    if type(exec_argv) ~= "function" then
      error("github-devloop: git adapter requires exec_argv")
    end
    git_handle = require("std.git").new(exec_argv)
  end
  return git_handle
end

local function bounded_limit(value, fallback, minimum, maximum, message)
  local n = tonumber(value or fallback)
  if n == nil or n < minimum or n > maximum then
    error(message)
  end
  return math.floor(n)
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

local function validate_fields(fields, message)
  local selected_fields = tostring(fields or "")
  if selected_fields == "" or selected_fields:match("[^%w_,]") or selected_fields:match("^,") or selected_fields:match(",$") or selected_fields:match(",,") then
    error(message)
  end
  return selected_fields
end

local function require_safe_branch(name, value)
  if not M._is_git_ref_safe(value) then
    error("github-devloop: invalid " .. tostring(name))
  end
  return tostring(value)
end

local function require_safe_remote(remote)
  local value = tostring(remote or "")
  if value == "" or value:find("[\r\n]") ~= nil then
    error("github-devloop: invalid git remote")
  end
  if not M._is_git_ref_safe(value) then
    error("github-devloop: invalid git remote")
  end
  return value
end

local function require_safe_sha(name, value)
  if not M._is_git_sha(value) then
    error("github-devloop: invalid " .. tostring(name))
  end
  return tostring(value)
end

local function require_positive_pr_number(value)
  if not M._is_positive_pr_number(value) then
    error("github-devloop: invalid pull request number")
  end
  return tostring(value)
end

local function require_label_name(name)
  local value = tostring(name or "")
  if value == "" then
    error("github-devloop: label name is required")
  end
  return value
end

local function require_label_color(color)
  local value = tostring(color or "")
  if value:find("^%x%x%x%x%x%x$") == nil then
    error("github-devloop: label color is invalid")
  end
  return value
end

local function require_dashboard_label(label)
  local value = tostring(label or "")
  if value == "" then
    error("github-devloop: dashboard issue label is required")
  end
  return value
end

local function worktree_parent_dir(worktree)
  local value = tostring(worktree or "")
  if value == "" or value:find("[\r\n]") ~= nil then
    error("github-devloop: invalid worktree path")
  end
  return value:gsub("/+$", ""):match("^(.*)/[^/]+$") or "."
end

local function run_mkdir(path, timeout)
  local result = exec_sync({ cmd = M.mkdir_p_cmd(path), timeout = timeout or 30 })
  if result.exit_code ~= 0 then
    error("github-devloop: directory setup failed: " .. tostring(result.stderr))
  end
  return result
end

local function run_path_is_directory(path, timeout)
  return exec_sync({ cmd = M.path_is_directory_cmd(path), timeout = timeout or 30 })
end

function M.gh_issue_list_intake(repo, limit, timeout)
  return gh_result(function()
    return github().issue_list_intake(
      repo,
      bounded_limit(limit, 100, 1, 100, "github-devloop: invalid intake issue list limit"),
      timeout
    )
  end)
end

function M.gh_issue_list_intake_probe(repo, limit, since, timeout)
  return gh_result(function()
    return github().issue_list_intake_probe(
      repo,
      bounded_limit(limit, 5, 1, 10, "github-devloop: invalid intake probe issue list limit"),
      since,
      timeout
    )
  end)
end

function M.gh_issue_list_decompose_children(repo, proposal_id, timeout)
  return gh_result(function()
    return github().issue_search(
      repo,
      "fkst:github-devloop:decompose-child:v1 " .. tostring(proposal_id),
      "number,title,state,author,body,url",
      timeout
    )
  end)
end

function M.gh_issue_list_recent_closed(repo, limit, timeout)
  return gh_result(function()
    return github().issue_list_recent_closed(
      repo,
      bounded_limit(limit, 30, 1, 100, "github-devloop: invalid closed issue list limit"),
      timeout
    )
  end)
end

function M.gh_issue_list_board_digest(repo, timeout)
  return gh_result(function()
    return github().issue_list_board_digest(repo, timeout)
  end)
end

function M.gh_pr_list_board_digest(repo, timeout)
  return gh_result(function()
    return github().pr_list_board_digest(repo, timeout)
  end)
end

function M.gh_issue_list_observe(repo, label, page, include_headers, timeout)
  return gh_result(function()
    return github().issue_list_observe(repo, label, bounded_page_number(page), include_headers, timeout or observe_list_timeout)
  end)
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
    run = function(timeout)
      return M.gh_issue_list_observe(repo, label, page, include_headers, timeout)
    end,
    timeout = observe_list_timeout,
    read_coalesce = M.gh_issue_list_observe_read_coalesce(repo, label, page),
  }
end

function M.gh_issue_list_wip(repo, timeout)
  return gh_result(function()
    return github().issue_list_cli(repo, "open", 100, "number", timeout)
  end)
end

function M.gh_dashboard_issue_list(repo, label, timeout)
  local selected_label = require_dashboard_label(label)
  return gh_result(function()
    return github().api_paginate_slurp(
      "repos/" .. tostring(repo) .. "/issues?state=open&labels=" .. selected_label:gsub(":", "%%3A") .. "&per_page=100",
      timeout
    )
  end)
end

function M.gh_dashboard_issue_all_open(repo, timeout)
  return gh_result(function()
    return github().api_paginate_slurp("repos/" .. tostring(repo) .. "/issues?state=open&per_page=100", timeout)
  end)
end

function M.gh_dashboard_issue_add_label(repo, issue_number, label, timeout)
  local selected_label = require_dashboard_label(label)
  return gh_result(function()
    return github().api_method(
      "POST",
      "repos/" .. tostring(repo) .. "/issues/" .. tostring(issue_number) .. "/labels",
      { "labels[]=" .. selected_label },
      nil,
      nil,
      timeout
    )
  end)
end

function M.gh_dashboard_label_get(repo, label, timeout)
  local selected_label = require_dashboard_label(label)
  return gh_result(function()
    return github().api_method("GET", "repos/" .. tostring(repo) .. "/labels/" .. selected_label:gsub(":", "%%3A"), nil, nil, nil, timeout)
  end)
end

function M.gh_dashboard_label_create(repo, label, timeout)
  local selected_label = require_dashboard_label(label)
  return gh_result(function()
    return github().api_method("POST", "repos/" .. tostring(repo) .. "/labels", {
      "name=" .. selected_label,
      "color=ededed",
      "description=fkst observability dashboard singleton",
    }, nil, nil, timeout)
  end)
end

function M.gh_repo_labels_list(repo, timeout)
  return gh_result(function()
    return github().api_paginate_slurp("repos/" .. tostring(repo) .. "/labels?per_page=100", timeout)
  end)
end

function M.gh_repo_label_create(repo, name, color, description, timeout)
  return gh_result(function()
    return github().label_rest_create(repo, require_label_name(name), require_label_color(color), description, timeout)
  end)
end

function M.gh_repo_label_update(repo, name, color, description, timeout)
  return gh_result(function()
    return github().label_rest_update(repo, require_label_name(name), require_label_color(color), description, timeout)
  end)
end

function M.gh_dashboard_issue_create(repo, input_file, timeout)
  return gh_result(function()
    return github().api_method("POST", "repos/" .. tostring(repo) .. "/issues", nil, input_file, nil, timeout)
  end)
end

function M.gh_dashboard_issue_get(repo, issue_number, timeout)
  return gh_result(function()
    return github().api_method("GET", "repos/" .. tostring(repo) .. "/issues/" .. tostring(issue_number), nil, nil, true, timeout)
  end)
end

function M.gh_dashboard_issue_update(repo, issue_number, input_file, timeout)
  return gh_result(function()
    return github().api_method("PATCH", "repos/" .. tostring(repo) .. "/issues/" .. tostring(issue_number), nil, input_file, nil, timeout)
  end)
end

function M.gh_pr_list_observe(repo, page, include_headers, timeout)
  return gh_result(function()
    return github().pr_list_observe(repo, bounded_page_number(page), include_headers, timeout or observe_list_timeout)
  end)
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
    run = function(timeout)
      return M.gh_pr_list_observe(repo, page, include_headers, timeout)
    end,
    timeout = observe_list_timeout,
    read_coalesce = M.gh_pr_list_observe_read_coalesce(repo, page),
  }
end

function M.gh_pr_list_freshness(repo, timeout)
  return gh_result(function()
    return github().pr_list(repo, timeout)
  end)
end

function M.gh_pr_list_merge_queue(repo, base, timeout)
  return gh_result(function()
    return github().pr_list_merge_queue(repo, require_safe_branch("merge queue base branch", base), timeout)
  end)
end

function M.gh_issue_view(repo, issue_number, fields, timeout, run)
  return gh_result(function()
    return github(run).issue_view(repo, issue_number, validate_fields(fields, "github-devloop: invalid issue view fields"), timeout)
  end)
end

function M.gh_issue_view_intake_scan(repo, issue_number, timeout)
  return M.gh_issue_view(repo, issue_number, "title,labels,comments,state,assignees,author", timeout)
end

function M.gh_issue_view_intake_judge(repo, issue_number, timeout)
  return M.gh_issue_view(repo, issue_number, "title,body,updatedAt,labels,comments,state,assignees,author", timeout)
end

function M.gh_issue_view_state(repo, issue_number, timeout)
  return M.gh_issue_view(repo, issue_number, "title,updatedAt,labels,state,comments,assignees,author", timeout)
end

function M.gh_issue_view_claim(repo, issue_number, timeout)
  return M.gh_issue_view(repo, issue_number, "assignees,author", timeout)
end

function M.gh_issue_view_result(repo, issue_number, timeout)
  return M.gh_issue_view(repo, issue_number, "labels,comments", timeout)
end

function M.gh_issue_view_loop(repo, issue_number, timeout)
  return M.gh_issue_view(repo, issue_number, "title,updatedAt,labels,comments,state", timeout)
end

function M.gh_issue_view_meta(repo, issue_number, timeout)
  return M.gh_issue_view(repo, issue_number, "title,labels,comments", timeout)
end

function M.gh_issue_view_implement(repo, issue_number, timeout)
  return M.gh_issue_view(repo, issue_number, "title,body,labels,comments,state,author", timeout)
end

function M.gh_issue_view_open_pr(repo, issue_number, timeout)
  return M.gh_issue_view(repo, issue_number, "title,labels,comments,assignees,author", timeout)
end

function M.gh_issue_view_reviewing(repo, issue_number, timeout)
  return M.gh_issue_view(repo, issue_number, "labels,comments", timeout)
end

function M.gh_issue_view_review(repo, issue_number, timeout)
  return M.gh_issue_view(repo, issue_number, "title,labels,comments,assignees,author", timeout)
end

function M.gh_issue_view_decompose(repo, issue_number, timeout)
  return M.gh_issue_view(repo, issue_number, "title,body,labels,comments", timeout)
end

function M.gh_issue_view_fix(repo, issue_number, timeout)
  return M.gh_issue_view(repo, issue_number, "title,labels,comments", timeout)
end

function M.gh_issue_view_commit_subject(repo, issue_number, timeout)
  return M.gh_issue_view(repo, issue_number, "number,title", timeout)
end

function M.gh_issue_view_review_loop(repo, issue_number, timeout)
  return M.gh_issue_view(repo, issue_number, "title,labels,comments,assignees,author", timeout)
end

function M.gh_issue_view_merge(repo, issue_number, timeout)
  return M.gh_issue_view(repo, issue_number, "title,labels,comments,state,assignees", timeout)
end

function M.gh_issue_view_observe(repo, issue_number, timeout)
  return M.gh_issue_view(repo, issue_number, "title,comments,state,stateReason,assignees,author", timeout)
end

function M.gh_pr_view_origin(repo, pr_number, timeout)
  return gh_result(function()
    return github().pr_cli_view(
      repo,
      pr_number,
      "headRefName,headRefOid,baseRefName,state,updatedAt,mergedAt,comments,labels,mergeable,mergeStateStatus",
      timeout
    )
  end)
end

function M.gh_pr_view_observe(repo, pr_number, timeout)
  return M.gh_pr_view_origin(repo, pr_number, timeout)
end

function M.gh_pr_view_fix(repo, pr_number, timeout)
  return gh_result(function()
    return github().pr_cli_view(repo, pr_number, "headRefName,headRefOid,baseRefName,state,comments,headRepository,headRepositoryOwner,isCrossRepository", timeout)
  end)
end

function M.gh_pr_view_fix_precheck(repo, pr_number, timeout)
  return gh_result(function()
    return github().pr_cli_view(repo, pr_number, "headRefName,headRefOid,baseRefName,state,updatedAt,comments,headRepository,headRepositoryOwner,isCrossRepository", timeout)
  end)
end

function M.gh_pr_view_merge(repo, pr_number, timeout)
  return gh_result(function()
    return github().pr_cli_view(repo, pr_number, "headRefName,headRefOid,baseRefName,baseRefOid,state,updatedAt,isDraft,mergedAt,comments,headRepository,headRepositoryOwner,isCrossRepository,mergeable,mergeStateStatus,statusCheckRollup", timeout)
  end)
end

function M.gh_pr_view_freshness(repo, pr_number, timeout)
  return gh_result(function()
    return github().pr_cli_view(repo, pr_number, "headRefName,headRefOid,baseRefName,state,updatedAt,isDraft,comments,labels,headRepository,headRepositoryOwner,isCrossRepository,mergeable,mergeStateStatus,statusCheckRollup", timeout)
  end)
end

function M.gh_pr_list_head_base(repo, head, base, timeout)
  return gh_result(function()
    return github().pr_list_head(
      repo,
      require_safe_branch("PR head branch", head),
      require_safe_branch("PR base branch", base),
      timeout
    )
  end)
end

function M.gh_pr_list_head(repo, head, timeout)
  return gh_result(function()
    return github().pr_list_head(repo, require_safe_branch("PR head branch", head), nil, timeout)
  end)
end

function M.gh_pr_create(repo, head, base, title, body_file, timeout)
  return gh_result(function()
    return github().pr_create(
      repo,
      require_safe_branch("PR head branch", head),
      require_safe_branch("PR base branch", base),
      title,
      body_file,
      timeout
    )
  end)
end

function M.gh_pr_merge(repo, pr_number, head_sha, timeout)
  if tostring(head_sha or "") == "" then
    error("github-devloop: invalid merge head sha")
  end
  return gh_result(function()
    return github().pr_merge(repo, pr_number, head_sha, timeout)
  end)
end

function M.gh_commit_check_runs(repo, head_sha, timeout)
  return gh_result(function()
    return github().api_get(repo, "commits/" .. require_safe_sha("commit check-runs head sha", head_sha) .. "/check-runs", timeout)
  end)
end

function M.gh_check_run_rerequest(repo, check_run_id, timeout)
  local id = tostring(check_run_id or "")
  if id == "" or id:find("[^0-9]") ~= nil then
    error("github-devloop: invalid check-run id")
  end
  return gh_result(function()
    return github().api_method("POST", "repos/" .. tostring(repo) .. "/check-runs/" .. id .. "/rerequest", nil, nil, nil, timeout)
  end)
end

function M.gh_issue_comment_get(repo, comment_id, timeout)
  if not M.is_safe_comment_id(comment_id) then
    error("github-devloop: invalid comment id")
  end
  return gh_result(function()
    return github().comment_get(repo, comment_id, timeout)
  end)
end

function M.gh_pr_ready(repo, pr_number, timeout)
  return gh_result(function()
    return github().pr_ready(repo, pr_number, timeout)
  end)
end

function M.gh_issue_comment(repo, issue_number, body_file, timeout)
  return gh_result(function()
    return github().issue_comment(repo, issue_number, body_file, timeout)
  end)
end

function M.gh_pr_comment(repo, pr_number, body_file, timeout)
  return gh_result(function()
    return github().pr_comment(repo, pr_number, body_file, timeout)
  end)
end

function M.gh_pr_close(repo, pr_number, timeout)
  return gh_result(function()
    return github().pr_close(repo, pr_number, timeout)
  end)
end

function M.gh_issue_close(repo, issue_number, timeout)
  return gh_result(function()
    return github().issue_close(repo, issue_number, timeout)
  end)
end

function M.gh_pr_diff(repo, pr_number, timeout, run)
  return gh_result(function()
    return github(run).pr_diff(repo, pr_number, timeout)
  end)
end

function M.gh_pr_diff_name_only(repo, pr_number, timeout, run)
  return gh_result(function()
    return github(run).pr_diff_name_only(repo, pr_number, timeout)
  end)
end

function M.gh_pr_view_head(repo, pr_number, timeout)
  return gh_result(function()
    return github().pr_cli_view(repo, pr_number, "headRefName,baseRefName,state", timeout)
  end)
end

function M.gh_pr_view_context(repo, pr_number, timeout, run)
  return gh_result(function()
    return github(run).pr_cli_view(repo, pr_number, "title,body,headRefName,headRefOid,baseRefName,state,updatedAt,comments,labels", timeout)
  end)
end

function M.git_status(worktree, timeout)
  return git().status_porcelain(worktree, timeout)
end

function M.git_add_all(worktree, timeout)
  return git().add_all(worktree, timeout)
end

local function require_commit_message(message)
  local bounded_message = tostring(message or "")
  if bounded_message == "" or #bounded_message > 200 then
    error("github-devloop: invalid git commit message")
  end
  return bounded_message
end

function M.git_commit(worktree, message, timeout)
  return git().commit_message(worktree, require_commit_message(message), timeout)
end

function M.git_empty_commit(worktree, message, timeout)
  return git().empty_commit_message(worktree, require_commit_message(message), timeout)
end

function M.git_current_branch(worktree, timeout)
  if worktree == nil then
    return git().current_branch(timeout)
  end
  return git().current_branch_worktree(worktree, timeout)
end

function M.git_head_sha(worktree, timeout)
  return git().head_sha(worktree, timeout)
end

function M.git_base_head(branch, timeout)
  return git().remote_branch_head("origin", require_safe_branch("base branch", branch), timeout)
end

function M.git_fetch_branch(remote, branch, timeout)
  return git().fetch_branch(require_safe_remote(remote), require_safe_branch("fetch branch", branch), timeout)
end

function M.git_ls_remote_branch(remote, branch, timeout)
  return git().ls_remote_branch(require_safe_remote(remote), require_safe_branch("remote branch", branch), timeout)
end

function M.git_fetch_remote_branch_to_tracking_ref(remote, branch, tracking_ref, timeout)
  return git().fetch_remote_branch_to_tracking_ref(
    require_safe_remote(remote),
    require_safe_branch("remote branch", branch),
    require_safe_branch("tracking ref", tracking_ref),
    timeout
  )
end

function M.git_rev_parse_ref_commit(ref, timeout)
  return git().rev_parse_ref_commit(require_safe_branch("ref", ref), timeout)
end

function M.git_fetch_pr_merge_ref(remote, pr_number, timeout)
  return git().fetch_ref(require_safe_remote(remote), "refs/pull/" .. require_positive_pr_number(pr_number) .. "/merge", timeout)
end

function M.git_fetch_pr_head_ref(remote, pr_number, timeout)
  return git().fetch_ref(require_safe_remote(remote), "refs/pull/" .. require_positive_pr_number(pr_number) .. "/head", timeout)
end

function M.git_fetch_head_commit(timeout)
  return git().fetch_head_commit(timeout)
end

function M.git_remote_branch_head(remote, branch, timeout)
  return git().remote_branch_head(require_safe_remote(remote), require_safe_branch("remote branch", branch), timeout)
end

function M.git_worktree_merge_no_edit(worktree, sha, timeout)
  return git().merge_no_edit(worktree, require_safe_sha("merge sha", sha), timeout)
end

function M.git_worktree_reset_hard(worktree, branch, timeout)
  return git().reset_hard_branch(worktree, require_safe_branch("reset branch", branch), timeout)
end

function M.git_worktree_clean(worktree, timeout)
  return git().clean_fd(worktree, timeout)
end

function M.git_ahead_count(upstream, integration, timeout)
  return git().remote_ahead_count(
    require_safe_branch("upstream branch", upstream),
    require_safe_branch("integration branch", integration),
    timeout
  )
end

function M.git_show_ref_branch(branch, timeout)
  return git().show_ref_branch_quiet(require_safe_branch("branch", branch), timeout)
end

function M.git_show_ref(worktree, branch, timeout)
  return git().show_ref_worktree_branch_quiet(worktree, require_safe_branch("branch", branch), timeout)
end

function M.git_branch_ahead_count(base, branch, timeout)
  return git().branch_ahead_count(require_safe_sha("base head", base), require_safe_branch("branch", branch), timeout)
end

function M.git_branch_head(branch, timeout)
  return git().branch_head(require_safe_branch("branch", branch), timeout)
end

function M.git_push_branch(branch, timeout)
  return git().push_branch_plain(require_safe_branch("branch", branch), timeout)
end

function M.git_switch_branch(worktree, branch, timeout)
  return git().switch_branch(worktree, require_safe_branch("branch", branch), timeout)
end

function M.git_worktree_remove_if_present(worktree, timeout)
  local dir_result = run_path_is_directory(worktree, 30)
  if dir_result.exit_code == 1 then
    return { stdout = "", stderr = "", exit_code = 0 }
  end
  if dir_result.exit_code ~= 0 then
    return dir_result
  end
  return M.git_worktree_remove(worktree, timeout)
end

function M.git_worktree_force_clean(worktree, timeout)
  local value = tostring(worktree or "")
  if value == "" or value:find("[\r\n]") ~= nil then
    error("github-devloop: invalid worktree path")
  end
  M.git_worktree_remove(value, timeout)
  local prune = M.git_worktree_prune(timeout)
  if prune.exit_code ~= 0 then
    return prune
  end
  return { stdout = "", stderr = "", exit_code = 0 }
end

function M.git_worktree_add_new_branch(worktree, branch, base, timeout)
  run_mkdir(worktree_parent_dir(worktree), 30)
  return git().worktree_add_new_branch(worktree, require_safe_branch("branch", branch), require_safe_sha("base head", base), timeout)
end

function M.git_worktree_add_reset_branch(worktree, branch, base, timeout)
  run_mkdir(worktree_parent_dir(worktree), 30)
  return git().worktree_add_reset_branch(worktree, require_safe_branch("branch", branch), require_safe_sha("base head", base), timeout)
end

function M.git_worktree_add_existing_branch(worktree, branch, timeout)
  run_mkdir(worktree_parent_dir(worktree), 30)
  return git().worktree_add_existing_branch(worktree, require_safe_branch("branch", branch), timeout)
end

function M.git_worktree_add_remote_branch(worktree, remote, branch, force, timeout)
  run_mkdir(worktree_parent_dir(worktree), 30)
  return git().worktree_add_remote_branch(
    worktree,
    require_safe_remote(remote),
    require_safe_branch("branch", branch),
    force == true,
    timeout
  )
end

function M.git_worktree_list(timeout)
  return git().worktree_list(timeout)
end

function M.git_worktree_prune(timeout)
  return git().worktree_prune(timeout)
end

function M.git_rev_parse_branch(worktree, branch, timeout)
  return git().rev_parse_worktree_branch(worktree, require_safe_branch("branch", branch), timeout)
end

function M.read_runtime_root_cmd()
  return 'printf %s "$FKST_RUNTIME_ROOT"'
end

function M.mkdir_p_cmd(path)
  local value = tostring(path or "")
  if value == "" or value:find("[\r\n]") ~= nil then
    error("github-devloop: invalid directory path")
  end
  return "mkdir -p " .. M._shell_single_quote(value)
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
end

return S
