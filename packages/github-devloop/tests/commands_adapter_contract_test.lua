local h = require("tests.devloop_core_helpers")
local core = h.core
local t = h.t

local function assert_argv_equal(actual, expected)
  t.eq(#actual, #expected)
  for index, value in ipairs(expected) do
    t.eq(actual[index], value)
  end
end

local function with_exec_argv(fn)
  local old_exec_argv = exec_argv
  local calls = {}
  exec_argv = function(spec)
    table.insert(calls, spec)
    return { stdout = "", stderr = "", exit_code = 0 }
  end
  local ok, result = pcall(fn, calls)
  exec_argv = old_exec_argv
  if not ok then
    error(result)
  end
  return calls, result
end

return {
  test_commands_helpers_execute_github_via_argv_adapter = function()
    local calls = with_exec_argv(function()
      core.gh_issue_view_implement("owner/repo", 42, 31)
      core.gh_pr_merge("owner/repo", 7, "def456", 32)
      core.gh_check_run_rerequest("owner/repo", 123, 33)
    end)

    assert_argv_equal(calls[1].argv, {
      "gh",
      "issue",
      "view",
      "42",
      "--repo",
      "owner/repo",
      "--json",
      "title,body,labels,comments,state,author",
    })
    assert_argv_equal(calls[2].argv, {
      "gh",
      "pr",
      "merge",
      "7",
      "--repo",
      "owner/repo",
      "--merge",
      "--match-head-commit",
      "def456",
    })
    assert_argv_equal(calls[3].argv, {
      "gh",
      "api",
      "--method",
      "POST",
      "repos/owner/repo/check-runs/123/rerequest",
    })
    for index, call in ipairs(calls) do
      t.eq(call.argv[1], "gh")
      t.eq(call.timeout, index + 30)
      t.is_nil(call.cmd)
      t.is_nil(call.rate_pool)
    end
  end,

  test_commands_helpers_execute_git_via_argv_adapter = function()
    local calls = with_exec_argv(function()
      core.git_status("/tmp/wt", 41)
      core.git_branch_ahead_count("abc123", "feature/a", 42)
      t.mock_command(core.mkdir_p_cmd("/tmp"), { stdout = "", stderr = "", exit_code = 0 })
      core.git_worktree_add_remote_branch("/tmp/wt", "origin", "feature/a", true, 43)
      core.git_push_branch("feature/a", 44)
    end)

    assert_argv_equal(calls[1].argv, { "git", "-C", "/tmp/wt", "status", "--porcelain" })
    assert_argv_equal(calls[2].argv, { "git", "rev-list", "--count", "abc123..refs/heads/feature/a" })
    assert_argv_equal(calls[3].argv, {
      "git",
      "worktree",
      "add",
      "--force",
      "-B",
      "feature/a",
      "/tmp/wt",
      "refs/remotes/origin/feature/a",
    })
    assert_argv_equal(calls[4].argv, { "git", "push", "origin", "feature/a" })
    for index, call in ipairs(calls) do
      t.eq(call.argv[1], "git")
      t.eq(call.timeout, index + 40)
      t.is_nil(call.cmd)
      t.is_nil(call.rate_pool)
    end
  end,
}
