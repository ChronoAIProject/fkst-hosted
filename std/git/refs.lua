local M = {}

local function push_branch_argv(branch)
  return { "git", "push", "-u", "origin", tostring(branch) }
end

local function show_ref_branch_argv(branch)
  return { "git", "show-ref", "--verify", "refs/heads/" .. tostring(branch) }
end

local function is_ancestor_argv(maybe_ancestor_sha, descendant_sha)
  return { "git", "merge-base", "--is-ancestor", tostring(maybe_ancestor_sha), tostring(descendant_sha) }
end

local function fetch_branch_argv(remote, branch)
  return { "git", "fetch", tostring(remote), tostring(branch) }
end

local function remote_branch_head_argv(remote, branch)
  return { "git", "rev-parse", "--verify", "refs/remotes/" .. tostring(remote) .. "/" .. tostring(branch) .. "^{commit}" }
end

local function fetch_head_commit_argv()
  return { "git", "rev-parse", "--verify", "FETCH_HEAD^{commit}" }
end

local function rev_parse_verify_head_argv()
  return { "git", "rev-parse", "--verify", "HEAD" }
end

local function worktree_argv(worktree, ...)
  local argv = { "git", "-C", tostring(worktree) }
  for _, value in ipairs({ ... }) do
    table.insert(argv, tostring(value))
  end
  return argv
end

local function merge_no_ff_argv(worktree, sha)
  return worktree_argv(worktree, "merge", "--no-ff", "--no-commit", sha)
end

local function fast_forward_argv(worktree, sha)
  return worktree_argv(worktree, "merge", "--ff-only", sha)
end

local function remote_trees_equal_quiet_argv(upstream, integration)
  return {
    "git",
    "diff",
    "--quiet",
    "refs/remotes/origin/" .. tostring(upstream),
    "refs/remotes/origin/" .. tostring(integration),
  }
end

local function trees_equal_quiet_argv(sha_a, sha_b)
  return { "git", "diff", "--quiet", tostring(sha_a), tostring(sha_b) }
end

local function merge_tree_argv(approved_head_sha, base_head_sha)
  return { "git", "merge-tree", "--write-tree", tostring(approved_head_sha), tostring(base_head_sha) }
end

local function push_branch_force_with_lease_argv(branch, new_sha, expected_old_sha)
  local ref = "refs/heads/" .. tostring(branch)
  return {
    "git",
    "push",
    "origin",
    tostring(new_sha) .. ":" .. ref,
    "--force-with-lease=" .. ref .. ":" .. tostring(expected_old_sha),
  }
end

local function push_branch_update_argv(branch)
  return { "git", "push", "origin", "HEAD:refs/heads/" .. tostring(branch) }
end

local function push_worktree_branch_update_argv(worktree, branch, expected_old_sha)
  local ref = "refs/heads/" .. tostring(branch)
  local argv = worktree_argv(worktree, "push", "origin", "HEAD:" .. ref)
  if expected_old_sha ~= nil then
    table.insert(argv, "--force-with-lease=" .. ref .. ":" .. tostring(expected_old_sha))
  end
  return argv
end

local function unmerged_paths_argv(worktree)
  if worktree == nil then
    return { "git", "ls-files", "-u" }
  end
  return worktree_argv(worktree, "ls-files", "-u")
end

local function diff_check_argv(worktree, cached)
  local args = cached and { "diff", "--cached", "--check" } or { "diff", "--check" }
  if worktree == nil then
    table.insert(args, 1, "git")
    return args
  end
  if cached then
    return worktree_argv(worktree, "diff", "--cached", "--check")
  end
  return worktree_argv(worktree, "diff", "--check")
end

local function conflict_markers_argv(worktree)
  local pattern = "^(" .. string.rep("<", 7) .. "|" .. string.rep("=", 7) .. "|" .. string.rep(">", 7) .. ")"
  if worktree == nil then
    return { "git", "grep", "-n", "-I", "-E", pattern, "--", "." }
  end
  return worktree_argv(worktree, "grep", "-n", "-I", "-E", pattern, "--", ".")
end

local function commit_message_file_argv(worktree, message_file)
  return worktree_argv(worktree, "commit", "-F", message_file)
end

local function worktree_add_detached_argv(worktree, sha)
  return { "git", "worktree", "add", "--detach", tostring(worktree), tostring(sha) }
end

local function worktree_remove_argv(worktree)
  return { "git", "worktree", "remove", "--force", tostring(worktree) }
end

local function exec_result(handle, argv, timeout, context)
  local ok, result_or_error = pcall(handle._exec, argv, timeout, context)
  if ok then
    return result_or_error
  end
  if type(result_or_error) == "table" and result_or_error.result ~= nil then
    return result_or_error.result
  end
  error(result_or_error)
end

function M.install(handle)
  function handle.push_branch(branch, timeout)
    return exec_result(handle, push_branch_argv(branch), timeout, "git push")
  end

  function handle.show_ref_branch(branch, timeout)
    return exec_result(handle, show_ref_branch_argv(branch), timeout, "git show-ref")
  end

  function handle.is_ancestor(maybe_ancestor_sha, descendant_sha, timeout)
    return exec_result(handle, is_ancestor_argv(maybe_ancestor_sha, descendant_sha), timeout, "git merge-base --is-ancestor")
  end

  function handle.fetch_branch(remote, branch, timeout)
    return exec_result(handle, fetch_branch_argv(remote, branch), timeout, "git fetch")
  end

  function handle.remote_branch_head(remote, branch, timeout)
    return exec_result(handle, remote_branch_head_argv(remote, branch), timeout, "git rev-parse remote branch")
  end

  function handle.fetch_head_commit(timeout)
    return exec_result(handle, fetch_head_commit_argv(), timeout, "git rev-parse FETCH_HEAD")
  end

  function handle.rev_parse_verify_head(timeout)
    return exec_result(handle, rev_parse_verify_head_argv(), timeout, "git rev-parse --verify HEAD")
  end

  function handle.merge_no_ff(worktree, sha, timeout)
    return exec_result(handle, merge_no_ff_argv(worktree, sha), timeout, "git merge --no-ff")
  end

  function handle.fast_forward(worktree, sha, timeout)
    return exec_result(handle, fast_forward_argv(worktree, sha), timeout, "git merge --ff-only")
  end

  function handle.remote_trees_equal_quiet(upstream, integration, timeout)
    return exec_result(handle, remote_trees_equal_quiet_argv(upstream, integration), timeout, "git diff --quiet remote trees")
  end

  function handle.trees_equal_quiet(sha_a, sha_b, timeout)
    return exec_result(handle, trees_equal_quiet_argv(sha_a, sha_b), timeout, "git diff --quiet trees")
  end

  function handle.merge_tree(approved_head_sha, base_head_sha, timeout)
    return exec_result(handle, merge_tree_argv(approved_head_sha, base_head_sha), timeout, "git merge-tree --write-tree")
  end

  function handle.push_branch_force_with_lease(branch, new_sha, expected_old_sha, timeout)
    return exec_result(handle, push_branch_force_with_lease_argv(branch, new_sha, expected_old_sha), timeout, "git push --force-with-lease")
  end

  function handle.push_branch_update(branch, timeout)
    return exec_result(handle, push_branch_update_argv(branch), timeout, "git push branch update")
  end

  function handle.push_worktree_branch_update(worktree, branch, expected_old_sha, timeout)
    return exec_result(handle, push_worktree_branch_update_argv(worktree, branch, expected_old_sha), timeout, "git worktree push")
  end

  function handle.unmerged_paths(worktree, timeout)
    return exec_result(handle, unmerged_paths_argv(worktree), timeout, "git ls-files -u")
  end

  function handle.diff_check(worktree, cached, timeout)
    return exec_result(handle, diff_check_argv(worktree, cached), timeout, "git diff --check")
  end

  function handle.conflict_markers(worktree, timeout)
    return exec_result(handle, conflict_markers_argv(worktree), timeout, "git grep conflict markers")
  end

  function handle.commit_message_file(worktree, message_file, timeout)
    return exec_result(handle, commit_message_file_argv(worktree, message_file), timeout, "git commit -F")
  end

  function handle.worktree_add_detached(worktree, sha, timeout)
    return exec_result(handle, worktree_add_detached_argv(worktree, sha), timeout, "git worktree add --detach")
  end

  function handle.worktree_remove(worktree, timeout)
    return exec_result(handle, worktree_remove_argv(worktree), timeout, "git worktree remove")
  end
end

return M
