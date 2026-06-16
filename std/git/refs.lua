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

local function rev_parse_verify_head_argv()
  return { "git", "rev-parse", "--verify", "HEAD" }
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

  function handle.rev_parse_verify_head(timeout)
    return exec_result(handle, rev_parse_verify_head_argv(), timeout, "git rev-parse --verify HEAD")
  end
end

return M
