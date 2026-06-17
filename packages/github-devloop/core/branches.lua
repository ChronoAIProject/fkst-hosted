local S = {}
local strings = require("std.strings")
local decimal_checksum = strings.decimal_checksum

function S.install(M)
local function require_safe_branch(name, branch)
  if not M._is_git_ref_safe(branch) then
    error("github-devloop: invalid " .. tostring(name))
  end
  return tostring(branch)
end

local function require_safe_sha(name, sha)
  if not M._is_git_sha(sha) then
    error("github-devloop: invalid " .. tostring(name))
  end
  return tostring(sha)
end

local function require_safe_repo(repo)
  local value = tostring(repo or "")
  if value == "" or M.safe_repo(value) ~= value then
    error("github-devloop: invalid branch sync repo")
  end
  return value
end

local function require_sync_result(result)
  if result ~= "clean" and result ~= "resolved" then
    error("github-devloop: invalid branch sync result")
  end
  return result
end

local function runtime_root_path(runtime_root)
  local root = M._trim(runtime_root)
  if root == "" or root:find("[\r\n]") ~= nil then
    error("github-devloop: invalid FKST_RUNTIME_ROOT")
  end
  return root:gsub("/+$", "")
end

local git_handle = nil

local function git()
  if git_handle == nil then
    if type(exec_argv) ~= "function" then
      error("github-devloop: git adapter requires exec_argv")
    end
    git_handle = require("std.git").new(exec_argv)
  end
  return git_handle
end

local function run_git(fn, label)
  local ok, result_or_error = pcall(fn)
  if ok then
    return result_or_error
  end
  if type(result_or_error) == "table" and result_or_error.result ~= nil then
    return result_or_error.result
  end
  error(tostring(label or "git-adapter operation") .. " failed: " .. tostring(result_or_error))
end

local function run_git_ok(fn, label)
  local result = run_git(fn, label)
  if result.exit_code ~= 0 then
    return nil, tostring(label or "git-adapter operation") .. " failed: " .. tostring(result.stderr)
  end
  return result
end

function M.git_is_ancestor(maybe_ancestor_sha, descendant_sha, timeout)
  return git().is_ancestor(
    require_safe_sha("ancestor sha", maybe_ancestor_sha),
    require_safe_sha("descendant sha", descendant_sha),
    timeout
  )
end

function M.git_merge_no_ff(worktree, sha, timeout)
  return git().merge_no_ff(worktree, require_safe_sha("merge sha", sha), timeout)
end

function M.git_fast_forward(worktree, sha, timeout)
  return git().fast_forward(worktree, require_safe_sha("fast-forward sha", sha), timeout)
end

function M.git_remote_trees_equal_quiet(upstream, integration, timeout)
  return git().remote_trees_equal_quiet(
    require_safe_branch("upstream branch", upstream),
    require_safe_branch("integration branch", integration),
    timeout
  )
end

function M.git_trees_equal_quiet(sha_a, sha_b, timeout)
  return git().trees_equal_quiet(
    require_safe_sha("tree compare sha", sha_a),
    require_safe_sha("tree compare sha", sha_b),
    timeout
  )
end

function M.current_base_head(base_branch)
  local branch = require_safe_branch("base branch", base_branch)
  local fetch_result, fetch_error = run_git_ok(function()
    return git().fetch_branch("origin", branch, 60)
  end, "base fetch")
  if fetch_result == nil then
    return nil, fetch_error
  end
  local head_result, head_error = run_git_ok(function()
    return git().remote_branch_head("origin", branch, 30)
  end, "base head")
  if head_result == nil then
    return nil, head_error
  end
  local base_head = tostring(head_result.stdout or ""):gsub("%s+$", "")
  if not M.is_safe_head_sha(base_head) then
    return nil, "unsafe base head"
  end
  return base_head
end

function M.has_empty_resolution_delta(approved_head_sha, base_head_sha, new_head_sha)
  local approved = require_safe_sha("approved head sha", approved_head_sha)
  local base = require_safe_sha("base head sha", base_head_sha)
  local new_head = require_safe_sha("new head sha", new_head_sha)
  local merge_tree = git().merge_tree(approved, base, 120)
  local tree = tostring(merge_tree.stdout or ""):gsub("%s+$", "")
  if tree == "" then
    return false, "merge-tree produced no tree"
  end
  local result = git().trees_equal_quiet(tree, new_head, 30)
  if result.exit_code == 0 then
    return true, "empty"
  end
  return false, tostring(result.stderr or "")
end

function M.current_branch_head_sha(branch)
  local safe_branch = require_safe_branch("branch", branch)
  local fetch_result = git().fetch_branch("origin", safe_branch, 60)
  if fetch_result.exit_code ~= 0 then
    return nil
  end
  local head_result = git().fetch_head_commit(30)
  if head_result.exit_code ~= 0 then
    return nil
  end
  local head_sha = tostring(head_result.stdout or ""):gsub("%s+$", "")
  if not M.is_safe_head_sha(head_sha) then
    error("github-devloop: unsafe PR origin branch head sha")
  end
  return head_sha
end

function M.git_push_branch_force_with_lease(branch, new_sha, expected_old_sha, timeout)
  return git().push_branch_force_with_lease(
    require_safe_branch("push branch", branch),
    require_safe_sha("new branch sha", new_sha),
    require_safe_sha("expected old branch sha", expected_old_sha),
    timeout
  )
end

function M.git_push_branch_update(branch, timeout)
  return git().push_branch_update(require_safe_branch("push branch", branch), timeout)
end

function M.git_push_worktree_branch_update(worktree, branch, timeout)
  return git().push_worktree_branch_update(worktree, require_safe_branch("push branch", branch), nil, timeout)
end

function M.git_push_worktree_branch_update_with_lease(worktree, branch, expected_old_sha, timeout)
  return git().push_worktree_branch_update(
    worktree,
    require_safe_branch("push branch", branch),
    require_safe_sha("expected old branch sha", expected_old_sha),
    timeout
  )
end

function M.git_unmerged_paths(worktree, timeout)
  return git().unmerged_paths(worktree, timeout)
end

function M.git_diff_check(worktree, timeout)
  return git().diff_check(worktree, false, timeout)
end

function M.git_diff_cached_check(worktree, timeout)
  return git().diff_check(worktree, true, timeout)
end

function M.git_conflict_markers(worktree, timeout)
  return git().conflict_markers(worktree, timeout)
end

function M.git_commit_message_file(worktree, message_file, timeout)
  return git().commit_message_file(worktree, message_file, timeout)
end

function M.git_worktree_add_detached_plan(worktree, sha)
  local value = tostring(worktree or "")
  if value == "" or value:find("[\r\n]") ~= nil then
    error("github-devloop: invalid worktree path")
  end
  return {
    parent_dir = value:gsub("/+$", ""):match("^(.*)/[^/]+$") or ".",
    worktree = value,
    sha = require_safe_sha("worktree base sha", sha),
  }
end

function M.git_worktree_add_detached(worktree, sha, timeout)
  local plan = M.git_worktree_add_detached_plan(worktree, sha)
  return git().worktree_add_detached(plan.worktree, plan.sha, timeout)
end

function M.git_worktree_remove(worktree, timeout)
  return git().worktree_remove(worktree, timeout)
end

function M.branch_sync_lock_key(repo, upstream, integration)
  local key = "github-devloop/branch-sync/"
    .. M.safe_repo(require_safe_repo(repo))
    .. "/"
    .. require_safe_branch("upstream branch", upstream)
    .. "/"
    .. require_safe_branch("integration branch", integration)
  if not M._is_path_safe_key(key, M._max_key_len) then
    error("github-devloop: invalid branch sync lock key")
  end
  return key
end

function M.repo_ref_store_lock_key(repo)
  local key = "github-devloop/git/"
    .. M.safe_repo(require_safe_repo(repo))
    .. "/fetch"
  if not M._is_path_safe_key(key, M._max_key_len) then
    error("github-devloop: invalid git ref-store lock key")
  end
  return key
end

function M.with_repo_ref_store_lock(repo, fn)
  return with_lock(M.repo_ref_store_lock_key(repo), fn)
end

function M.pr_freshness_lock_key(repo, branch)
  local key = "github-devloop/pr-freshness/"
    .. M.safe_repo(require_safe_repo(repo))
    .. "/"
    .. require_safe_branch("managed branch", branch)
  if not M._is_path_safe_key(key, M._max_key_len) then
    error("github-devloop: invalid PR freshness lock key")
  end
  return key
end

function M.rollup_lock_key(repo, upstream, integration)
  local key = "github-devloop/rollup/"
    .. M.safe_repo(require_safe_repo(repo))
    .. "/"
    .. require_safe_branch("upstream branch", upstream)
    .. "/"
    .. require_safe_branch("integration branch", integration)
  if not M._is_path_safe_key(key, M._max_key_len) then
    error("github-devloop: invalid rollup lock key")
  end
  return key
end

function M.rollup_source_ref(repo, pr_number)
  return {
    kind = "external",
    ref = require_safe_repo(repo) .. "#pr/" .. tostring(pr_number),
  }
end

function M.rollup_dedup_key(repo, upstream, integration, pr_number, head_sha)
  return M._dedup_key({
    "rollup",
    require_safe_repo(repo),
    require_safe_branch("upstream branch", upstream),
    require_safe_branch("integration branch", integration),
    tostring(pr_number),
    require_safe_sha("head sha", head_sha),
  })
end

function M.rollup_ready_payload(repo, upstream, integration, pr_number, head_sha)
  return {
    schema = "github-devloop.v1",
    repo = require_safe_repo(repo),
    pr_number = pr_number,
    upstream_branch = require_safe_branch("upstream branch", upstream),
    integration_branch = require_safe_branch("integration branch", integration),
    head_sha = require_safe_sha("head sha", head_sha),
    dedup_key = M.rollup_dedup_key(repo, upstream, integration, pr_number, head_sha),
    source_ref = M.rollup_source_ref(repo, pr_number),
  }
end

function M.is_supported_rollup_ready(payload)
  if type(payload) ~= "table"
    or payload.schema ~= "github-devloop.v1"
    or not M._is_git_ref_safe(payload.upstream_branch)
    or not M._is_git_ref_safe(payload.integration_branch)
    or not M._is_git_sha(payload.head_sha)
    or type(payload.source_ref) ~= "table"
    or payload.source_ref.kind ~= "external" then
    return false
  end
  local ok = pcall(function()
    require_safe_repo(payload.repo)
  end)
  if not ok then
    return false
  end
  if tostring(payload.source_ref.ref or "") ~= M.rollup_source_ref(payload.repo, payload.pr_number).ref then
    return false
  end
  return tostring(payload.dedup_key or "") == M.rollup_dedup_key(
    payload.repo,
    payload.upstream_branch,
    payload.integration_branch,
    payload.pr_number,
    payload.head_sha
  )
end

function M.branch_sync_source_ref(repo, upstream, integration)
  return {
    kind = "external",
    ref = require_safe_repo(repo)
      .. "#branch-sync/"
      .. require_safe_branch("upstream branch", upstream)
      .. "/"
      .. require_safe_branch("integration branch", integration),
  }
end

function M.branch_sync_dedup_key(repo, upstream, integration, upstream_sha)
  return M._dedup_key({
    "branch-sync",
    require_safe_repo(repo),
    require_safe_branch("upstream branch", upstream),
    require_safe_branch("integration branch", integration),
    require_safe_sha("upstream sha", upstream_sha),
  })
end

function M.pr_freshness_dedup_key(repo, branch, baseline_sha)
  return M._dedup_key({
    "pr-freshness",
    require_safe_repo(repo),
    require_safe_branch("managed branch", branch),
    require_safe_sha("baseline sha", baseline_sha),
  })
end

function M.pr_freshness_source_ref(repo, pr_number)
  return {
    kind = "external",
    ref = require_safe_repo(repo) .. "#pr/" .. tostring(pr_number),
  }
end

function M.sync_commit_marker(repo, upstream, integration, upstream_sha, integration_parent, result)
  return '<!-- fkst:github-devloop:sync:v1 repo="' .. require_safe_repo(repo)
    .. '" upstream="' .. require_safe_branch("upstream branch", upstream)
    .. '" integration="' .. require_safe_branch("integration branch", integration)
    .. '" upstream_sha="' .. require_safe_sha("upstream sha", upstream_sha)
    .. '" integration_parent="' .. require_safe_sha("integration parent", integration_parent)
    .. '" result="' .. require_sync_result(result)
    .. '" -->'
end

function M.sync_commit_message(repo, upstream, integration, upstream_sha, integration_parent, result)
  return "Sync " .. require_safe_branch("upstream branch", upstream)
    .. " into " .. require_safe_branch("integration branch", integration)
    .. "\n\n"
    .. M.sync_commit_marker(repo, upstream, integration, upstream_sha, integration_parent, result)
end

function M.pr_freshness_commit_message(repo, branch, integration, branch_parent, integration_sha)
  return "Refresh " .. require_safe_branch("managed branch", branch)
    .. " from " .. require_safe_branch("integration branch", integration)
    .. "\n\n"
    .. "<!-- fkst:github-devloop:pr-freshness:v1 repo=\"" .. require_safe_repo(repo)
    .. "\" branch=\"" .. require_safe_branch("managed branch", branch)
    .. "\" integration=\"" .. require_safe_branch("integration branch", integration)
    .. "\" branch_parent=\"" .. require_safe_sha("branch parent", branch_parent)
    .. "\" integration_sha=\"" .. require_safe_sha("integration sha", integration_sha)
    .. "\" -->"
end

function M.branch_sync_worktree_path(runtime_root, repo, upstream, integration, integration_sha)
  local slug = strings.sanitize_key(
    require_safe_repo(repo)
      .. "-"
      .. require_safe_branch("upstream branch", upstream)
      .. "-"
      .. require_safe_branch("integration branch", integration),
    false
  ):gsub("/", "-")
  slug = slug:gsub("%-+", "-"):gsub("^%-+", ""):gsub("%-+$", "")
  if slug == "" then
    slug = "branch-sync"
  end
  if #slug > 90 then
    slug = slug:sub(1, 90):gsub("%-+$", "")
  end
  local suffix = decimal_checksum(
    require_safe_repo(repo)
      .. "#"
      .. require_safe_branch("upstream branch", upstream)
      .. "#"
      .. require_safe_branch("integration branch", integration)
      .. "#"
      .. require_safe_sha("integration sha", integration_sha)
  )
  return runtime_root_path(runtime_root) .. "/worktrees/sync-" .. slug .. "-" .. suffix
end

function M.branch_sync_message_file(runtime_root, repo, upstream, integration, upstream_sha, integration_sha)
  local suffix = decimal_checksum(
    require_safe_repo(repo)
      .. "#"
      .. require_safe_branch("upstream branch", upstream)
      .. "#"
      .. require_safe_branch("integration branch", integration)
      .. "#"
      .. require_safe_sha("upstream sha", upstream_sha)
      .. "#"
      .. require_safe_sha("integration sha", integration_sha)
  )
  runtime_root_path(runtime_root)
  return "/tmp/fkst-github-devloop-sync-message-" .. suffix .. ".txt"
end

function M.pr_freshness_message_file(runtime_root, repo, branch, integration, branch_parent, integration_sha)
  local suffix = decimal_checksum(
    require_safe_repo(repo)
      .. "#"
      .. require_safe_branch("managed branch", branch)
      .. "#"
      .. require_safe_branch("integration branch", integration)
      .. "#"
      .. require_safe_sha("branch parent", branch_parent)
      .. "#"
      .. require_safe_sha("integration sha", integration_sha)
  )
  runtime_root_path(runtime_root)
  return "/tmp/fkst-github-devloop-pr-freshness-message-" .. suffix .. ".txt"
end

function M.is_supported_sync_conflict(payload)
  if type(payload) ~= "table"
    or payload.schema ~= "github-devloop.v1"
    or not M._is_git_ref_safe(payload.upstream_branch)
    or not M._is_git_ref_safe(payload.integration_branch)
    or not M._is_git_sha(payload.upstream_sha)
    or not M._is_git_sha(payload.integration_sha)
    or type(payload.source_ref) ~= "table"
    or payload.source_ref.kind ~= "external" then
    return false
  end

  local ok = pcall(function()
    require_safe_repo(payload.repo)
  end)
  if not ok then
    return false
  end

  local expected_ref = M.branch_sync_source_ref(payload.repo, payload.upstream_branch, payload.integration_branch)
  local expected_branch_sync = tostring(payload.source_ref.ref or "") == expected_ref.ref
    and tostring(payload.dedup_key or "") == M.branch_sync_dedup_key(
    payload.repo,
    payload.upstream_branch,
    payload.integration_branch,
    payload.upstream_sha
  )
  if expected_branch_sync then
    return true
  end

  local expected_pr_freshness = tostring(payload.source_ref.ref or ""):match("^" .. require_safe_repo(payload.repo):gsub("([^%w])", "%%%1") .. "#pr/%d+$") ~= nil
    and tostring(payload.dedup_key or "") == M.pr_freshness_dedup_key(
      payload.repo,
      payload.integration_branch,
      payload.upstream_sha
    )
  return expected_pr_freshness
end
end

return S
