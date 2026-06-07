local S = {}

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

function M.git_is_ancestor_cmd(maybe_ancestor_sha, descendant_sha)
  return "git merge-base --is-ancestor "
    .. M._shell_single_quote(require_safe_sha("ancestor sha", maybe_ancestor_sha))
    .. " "
    .. M._shell_single_quote(require_safe_sha("descendant sha", descendant_sha))
end

function M.git_merge_no_ff_cmd(worktree, sha)
  if sha == nil then
    sha = worktree
    return "git merge --no-ff --no-commit " .. M._shell_single_quote(require_safe_sha("merge sha", sha))
  end
  return "git -C " .. M._shell_single_quote(worktree)
    .. " merge --no-ff --no-commit "
    .. M._shell_single_quote(require_safe_sha("merge sha", sha))
end

function M.git_fast_forward_cmd(worktree, sha)
  return "git -C " .. M._shell_single_quote(worktree)
    .. " merge --ff-only "
    .. M._shell_single_quote(require_safe_sha("fast-forward sha", sha))
end

function M.git_remote_trees_equal_quiet_cmd(upstream, integration)
  return "git diff --quiet refs/remotes/origin/"
    .. M._shell_single_quote(require_safe_branch("upstream branch", upstream))
    .. " refs/remotes/origin/"
    .. M._shell_single_quote(require_safe_branch("integration branch", integration))
end

function M.git_trees_equal_quiet_cmd(sha_a, sha_b)
  return "git diff --quiet "
    .. M._shell_single_quote(require_safe_sha("tree compare sha", sha_a))
    .. " "
    .. M._shell_single_quote(require_safe_sha("tree compare sha", sha_b))
end

function M.git_push_branch_force_with_lease_cmd(branch, new_sha, expected_old_sha)
  local safe_branch = require_safe_branch("push branch", branch)
  local safe_new_sha = require_safe_sha("new branch sha", new_sha)
  local safe_expected_old_sha = require_safe_sha("expected old branch sha", expected_old_sha)
  return "git push origin "
    .. M._shell_single_quote(safe_new_sha .. ":refs/heads/" .. safe_branch)
    .. " --force-with-lease="
    .. M._shell_single_quote("refs/heads/" .. safe_branch .. ":" .. safe_expected_old_sha)
end

function M.git_push_branch_update_cmd(branch)
  return "git push origin HEAD:refs/heads/" .. M._shell_single_quote(require_safe_branch("push branch", branch))
end

function M.git_push_worktree_branch_update_cmd(worktree, branch)
  return "git -C " .. M._shell_single_quote(worktree)
    .. " push origin HEAD:refs/heads/"
    .. M._shell_single_quote(require_safe_branch("push branch", branch))
end

function M.git_unmerged_paths_cmd(worktree)
  if worktree == nil then
    return "git ls-files -u"
  end
  return "git -C " .. M._shell_single_quote(worktree) .. " ls-files -u"
end

function M.git_diff_check_cmd(worktree)
  if worktree == nil then
    return "git diff --check"
  end
  return "git -C " .. M._shell_single_quote(worktree) .. " diff --check"
end

function M.git_diff_cached_check_cmd(worktree)
  if worktree == nil then
    return "git diff --cached --check"
  end
  return "git -C " .. M._shell_single_quote(worktree) .. " diff --cached --check"
end

function M.git_commit_message_file_cmd(worktree, message_file)
  return "git -C " .. M._shell_single_quote(worktree)
    .. " commit -F " .. M._shell_single_quote(message_file)
end

function M.git_worktree_add_detached_cmd(worktree, sha)
  return "mkdir -p " .. M._shell_single_quote(tostring(worktree):gsub("/+$", ""):match("^(.*)/[^/]+$") or ".")
    .. " && git worktree add --detach "
    .. M._shell_single_quote(worktree)
    .. " "
    .. M._shell_single_quote(require_safe_sha("worktree base sha", sha))
end

function M.git_worktree_remove_cmd(worktree)
  return "git worktree remove --force " .. M._shell_single_quote(worktree)
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

function M.branch_sync_worktree_path(runtime_root, repo, upstream, integration, integration_sha)
  local slug = M.sanitize_key(
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
  local suffix = M._decimal_checksum(
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
  local suffix = M._decimal_checksum(
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
  if tostring(payload.source_ref.ref or "") ~= expected_ref.ref then
    return false
  end
  return tostring(payload.dedup_key or "") == M.branch_sync_dedup_key(
    payload.repo,
    payload.upstream_branch,
    payload.integration_branch,
    payload.upstream_sha
  )
end
end

return S
