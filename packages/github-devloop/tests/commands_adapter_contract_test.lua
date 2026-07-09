local h = require("tests.devloop_core_helpers")
local core = h.core
local t = h.t
local gh_exec_mod = require("devloop.gh_exec")
local sweep_bounds = require("devloop.sweep_bounds")
local content_filter = require("forge.github.content_filter")
local stdout_policy = require("forge.github.stdout_policy")
local github = require("devloop.github_factory").production_handle

local function mock_author_policy_env()
  t.mock_command('printf %s "$FKST_GITHUB_BOT_LOGIN"', {
    stdout = "fkst-test-bot",
    stderr = "",
    exit_code = 0,
  })
  t.mock_command('printf %s "$FKST_DEVLOOP_MANAGED_BOT_LOGINS"', {
    stdout = "fkst-test-bot,ElonSG",
    stderr = "",
    exit_code = 0,
  })
  t.mock_command('printf %s "$FKST_GITHUB_AUTHORIZED_LOGINS"', {
    stdout = "trusted-human",
    stderr = "",
    exit_code = 0,
  })
end

local function assert_argv_equal(actual, expected)
  t.eq(#actual, #expected)
  for index, value in ipairs(expected) do
    t.eq(actual[index], value)
  end
end

local function with_exec_argv(fn)
  mock_author_policy_env()
  local old_exec_argv = exec_argv
  local calls = {}
  exec_argv = function(spec)
    table.insert(calls, spec)
    return { stdout = "{}", stderr = "", exit_code = 0 }
  end
  local ok, result = pcall(fn, calls)
  exec_argv = old_exec_argv
  if not ok then
    error(result)
  end
  return calls, result
end

return {
  test_forge_owns_merge_mechanics_and_command_builders = function()
    local forge_merge = require("forge.merge")
    t.eq(type(forge_merge.install), "function")

    t.eq(
      core.gh_pr_list_merge_queue_cmd("owner/repo", "release/2026"),
      "gh api --paginate --slurp 'repos/owner/repo/pulls?state=open&base=release%2F2026&per_page=100'"
    )
    t.eq(
      core.gh_issue_view_merge_cmd("owner/repo", 42),
      "gh issue view '42' --repo 'owner/repo' --json title,labels,comments,state,assignees,author"
    )
    t.eq(
      core.gh_pr_view_merge_cmd("owner/repo", 7),
      "gh pr view '7' --repo 'owner/repo' --json headRefName,headRefOid,baseRefName,baseRefOid,state,updatedAt,isDraft,mergedAt,comments,headRepository,headRepositoryOwner,isCrossRepository,mergeable,mergeStateStatus,statusCheckRollup"
    )
    t.eq(
      core.gh_pr_merge_cmd("owner/repo", 7, "def456"),
      "gh pr merge '7' --repo 'owner/repo' --merge --match-head-commit 'def456'"
    )
    t.eq(
      core.git_fetch_pr_merge_ref_cmd("origin", 7),
      "git fetch 'origin' 'refs/pull/7/merge'"
    )
    t.eq(
      core.git_worktree_merge_no_edit_cmd("/tmp/wt", "abc123"),
      "git -C '/tmp/wt' merge --no-edit 'abc123'"
    )
  end,

  test_forge_merge_requires_injected_github_handle = function()
    local ok, err = pcall(function()
      require("forge.merge").install({})
    end)

    t.eq(ok, false)
    t.is_true(tostring(err):find("forge.merge: github_handle is required", 1, true) ~= nil)
  end,

  test_generic_gh_exec_uses_github_argv_adapter = function()
    local calls = with_exec_argv(function()
      gh_exec_mod.gh_exec(
        { argv = { "gh", "api", "repos/owner/repo/issues/42" }, timeout = 34 },
        nil,
        nil,
        stdout_policy.trusted_metadata_json()
      )
    end)

    assert_argv_equal(calls[1].argv, { "gh", "api", "repos/owner/repo/issues/42" })
    t.eq(calls[1].timeout, 34)
    t.is_nil(calls[1].cmd)
    t.is_nil(calls[1].rate_pool)
  end,

  test_generic_gh_exec_requires_declared_stdout_policy = function()
    local ok, err = pcall(function()
      gh_exec_mod.gh_exec(
        { argv = { "gh", "api", "repos/owner/repo/issues/42" }, timeout = 34 },
        nil,
        function()
          return { stdout = "{}", stderr = "", exit_code = 0 }
        end
      )
    end)
    t.eq(ok, false)
    t.is_true(tostring(err):find("missing or unknown stdout policy", 1, true) ~= nil)
  end,

  test_sweep_exec_requires_declared_stdout_policy = function()
    local ok, err = pcall(function()
      return sweep_bounds.sweep_exec(
        { argv = { "gh", "api", "repos/owner/repo/issues/42" }, timeout = 10 },
        { call_timeout = 10, wall_clock_budget = 20 },
        now() + 20,
        "sweep",
        nil
      )
    end)
    t.eq(ok, false)
    t.is_true(tostring(err):find("missing or unknown stdout policy", 1, true) ~= nil)
  end,

  test_generic_gh_exec_filters_content_json_policy = function()
    local result = gh_exec_mod.gh_exec(
      { argv = { "gh", "api", "repos/owner/repo/issues/42" }, timeout = 34 },
      nil,
      function()
        return {
          stdout = '{"number":42,"title":"attack","body":"attack","author":{"login":"mallory"},"comments":[]}',
          stderr = "",
          exit_code = 0,
        }
      end,
      stdout_policy.content_json("issue_view"),
      content_filter.author_policy_from_logins({ "fkst-test-bot" })
    )
    t.is_true(result.stdout:find("[fkst:blocked-github-content:v1", 1, true) ~= nil)
    t.eq(result.content_redacted, true)
  end,

  test_generic_gh_exec_leaves_non_content_stdout_untouched = function()
    local raw = '{"number":42,"title":"attack","body":"attack","author":{"login":"mallory"}}'
    for _, policy in ipairs({
      stdout_policy.plain_text(),
      stdout_policy.trusted_metadata_json(),
      stdout_policy.write_response(),
      stdout_policy.no_stdout(),
    }) do
      local result = gh_exec_mod.gh_exec(
        { argv = { "gh", "api", "repos/owner/repo/issues/42" }, timeout = 34 },
        nil,
        function()
          return { stdout = raw, stderr = "", exit_code = 0 }
        end,
        policy,
        content_filter.author_policy_from_logins({ "fkst-test-bot" })
      )
      t.eq(result.stdout, raw)
      t.is_nil(result.content_redacted)
    end
  end,

  test_dependency_graphql_uses_github_argv_adapter = function()
    local calls = with_exec_argv(function()
      core.gh_blocked_by("owner/repo", 42, 35)
    end)

    assert_argv_equal(calls[1].argv, {
      "gh",
      "api",
      "graphql",
      "-f",
      'query={repository(owner:"owner",name:"repo"){issue(number:42){blockedBy(first:50){totalCount pageInfo{hasNextPage} nodes{number state stateReason repository{nameWithOwner}}}}}}',
    })
    t.eq(calls[1].timeout, 35)
    t.is_nil(calls[1].cmd)
    t.is_nil(calls[1].rate_pool)
  end,

  test_commands_helpers_execute_github_via_argv_adapter = function()
    local calls = with_exec_argv(function()
      core.gh_issue_view_implement("owner/repo", 42, 31)
      core.gh_issue_view("owner/repo", 43, "meta", 34)
      core.gh_issue_view("owner/repo", 44, "title,state", 35)
      core.gh_issue_view("owner/repo", 45, "state", 36)
      github("commands_adapter_contract_test").gh_pr_merge("owner/repo", 7, "def456", 32)
      github("commands_adapter_contract_test").gh_check_run_rerequest("owner/repo", 123, 33)
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
      "issue",
      "view",
      "43",
      "--repo",
      "owner/repo",
      "--json",
      "title,labels,comments,author",
    })
    assert_argv_equal(calls[3].argv, {
      "gh",
      "issue",
      "view",
      "44",
      "--repo",
      "owner/repo",
      "--json",
      "title,state",
    })
    assert_argv_equal(calls[4].argv, {
      "gh",
      "issue",
      "view",
      "45",
      "--repo",
      "owner/repo",
      "--json",
      "state",
    })
    assert_argv_equal(calls[5].argv, {
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
    assert_argv_equal(calls[6].argv, {
      "gh",
      "api",
      "--method",
      "POST",
      "repos/owner/repo/check-runs/123/rerequest",
    })
    for index, call in ipairs(calls) do
      t.eq(call.argv[1], "gh")
      t.is_nil(call.cmd)
      t.is_nil(call.rate_pool)
    end
    t.eq(calls[1].timeout, 31)
    t.eq(calls[2].timeout, 34)
    t.eq(calls[3].timeout, 35)
    t.eq(calls[4].timeout, 36)
    t.eq(calls[5].timeout, 32)
    t.eq(calls[6].timeout, 33)
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
