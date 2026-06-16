local gh = require("std.github")
local git = require("std.git")

local function assert_argv_equal(actual, expected, context)
  assert(type(actual) == "table", context .. " argv must be a table")
  assert(#actual == #expected, context .. " argv length mismatch")
  for index, value in ipairs(expected) do
    assert(actual[index] == value, context .. " argv[" .. tostring(index) .. "] mismatch")
  end
end

local function issue_stdout()
  return [[{"number":42,"title":"Title","body":"Body","url":"https://github.com/owner/repo/issues/42","updatedAt":"2026-06-15T00:00:00Z","state":"OPEN","labels":[],"comments":[],"assignees":[],"author":{"login":"author"}}]]
end

return {
  test_exec_classifies_rate_limit = function()
    local handle = gh.new(function(_opts)
      return { stdout = "", stderr = "API rate limit exceeded for user", exit_code = 1 }
    end)
    local ok, err = pcall(function()
      return handle._exec({ "gh", "api", "x" }, 10, "ctx")
    end)
    assert(ok == false)
    assert(err.class == "gh-rate-limited", "rate-limit stderr must classify as gh-rate-limited")
    assert(err.retryable == true)
  end,

  test_exec_classifies_already_exceeded_rate_limit = function()
    -- Regression (#710 Finding 1): the dominant GitHub wording interposes
    -- "already", which a contiguous "api rate limit exceeded" needle misses,
    -- mis-classifying the most common rate-limit error as non-retryable.
    local handle = gh.new(function(_opts)
      return { stdout = "", stderr = "GraphQL: API rate limit already exceeded for user ID 1593871", exit_code = 1 }
    end)
    local ok, err = pcall(function()
      return handle._exec({ "gh", "api", "x" }, 10, "ctx")
    end)
    assert(ok == false)
    assert(err.class == "gh-rate-limited", "'already exceeded' wording must classify as gh-rate-limited")
    assert(err.retryable == true)
  end,

  test_exec_classifies_generic_failure = function()
    local handle = gh.new(function(_opts)
      return { stdout = "", stderr = "fatal: not found", exit_code = 1 }
    end)
    local ok, err = pcall(function()
      return handle._exec({ "gh", "api", "y" }, 10, "ctx")
    end)
    assert(ok == false)
    assert(err.class == "gh-command-failed")
  end,

  test_exec_returns_result_on_success = function()
    local handle = gh.new(function(_opts)
      return { stdout = "ok", stderr = "", exit_code = 0 }
    end)
    local out = handle._exec({ "gh", "api", "z" }, 10, "ctx")
    assert(out.stdout == "ok")
  end,

  test_github_exec_uses_argv_without_shell_fields = function()
    local seen
    local handle = gh.new(function(opts)
      seen = opts
      return { stdout = "ok", stderr = "", exit_code = 0 }
    end)

    handle._exec({ "gh", "api", "repos/owner/repo" }, 12, "ctx")

    assert_argv_equal(seen.argv, { "gh", "api", "repos/owner/repo" }, "github")
    assert(seen.timeout == 12, "timeout is forwarded")
    assert(seen.cmd == nil, "github exec must not pass cmd")
    assert(seen.rate_pool == nil, "github exec must not pass rate_pool")
  end,

  test_github_exec_rejects_non_gh_program = function()
    local handle = gh.new(function(_opts)
      error("exec must not be called for adapter misuse")
    end)

    local ok, err = pcall(function()
      return handle._exec({ "git", "api", "repos/owner/repo" }, 12, "ctx")
    end)

    assert(ok == false)
    assert(err.class == "gh-adapter-misuse")
    assert(err.bad_program == "git")
    assert(tostring(err):find("git", 1, true) ~= nil, "misuse error must name the bad program")
  end,

  test_git_exec_uses_argv_without_shell_fields = function()
    local seen
    local handle = git.new(function(opts)
      seen = opts
      return { stdout = "ok", stderr = "", exit_code = 0 }
    end)

    handle._exec({ "git", "status", "--short" }, 7, "ctx")

    assert_argv_equal(seen.argv, { "git", "status", "--short" }, "git")
    assert(seen.timeout == 7, "timeout is forwarded")
    assert(seen.cmd == nil, "git exec must not pass cmd")
    assert(seen.rate_pool == nil, "git exec must not pass rate_pool")
  end,

  test_git_exec_rejects_non_git_program = function()
    local handle = git.new(function(_opts)
      error("exec must not be called for adapter misuse")
    end)

    local ok, err = pcall(function()
      return handle._exec({ "gh", "status", "--short" }, 7, "ctx")
    end)

    assert(ok == false)
    assert(err.class == "git-adapter-misuse")
    assert(err.bad_program == "gh")
    assert(tostring(err):find("gh", 1, true) ~= nil, "misuse error must name the bad program")
  end,

  test_read_issue_builder_uses_gh_argv = function()
    local calls = {}
    local comments_query = table.concat({ "per", "page=100" }, "_")
    local comments_path = "repos/owner/repo/issues/42/comments?" .. comments_query
    local handle = gh.new(function(opts)
      table.insert(calls, opts)
      if opts.argv[5] == comments_path then
        return { stdout = "[]", stderr = "", exit_code = 0 }
      end
      return { stdout = issue_stdout(), stderr = "", exit_code = 0 }
    end)

    local issue = handle.read_issue({ kind = "external", ref = "owner/repo#issue/42" }, {
      force_fresh = true,
      timeout = 9,
    })

    assert(issue.number == 42, "read_issue still parses stdout")
    assert(#calls == 2, "force_fresh read_issue fetches REST issue and comments")
    assert_argv_equal(calls[1].argv, { "gh", "api", "repos/owner/repo/issues/42" }, "read_issue")
    assert_argv_equal(
      calls[2].argv,
      { "gh", "api", "--paginate", "--slurp", comments_path },
      "read_issue comments"
    )
    for index, call in ipairs(calls) do
      assert(call.timeout == 9, "read_issue forwards timeout for call " .. tostring(index))
      assert(call.cmd == nil, "read_issue must not pass cmd")
      assert(call.rate_pool == nil, "read_issue must not pass rate_pool")
    end
  end,

  test_github_entity_methods_build_argv = function()
    local calls = {}
    local handle = gh.new(function(opts)
      table.insert(calls, opts)
      return { stdout = "ok", stderr = "", exit_code = 0 }
    end)

    handle.issue_list("owner/repo", 11)
    handle.pr_list("owner/repo", 12)
    handle.pr_list_head("owner/repo", "feature/a", "dev", 13)
    handle.pr_view("owner/repo", 7, 14)
    handle.pr_create("owner/repo", "feature/a", "dev", "Fix title", "/tmp/body.md", 15)
    handle.label_list("owner/repo", 16)
    handle.label_create("owner/repo", "fkst-dev:ready", "0E8A16", 17)
    handle.issue_edit_labels("owner/repo", 42, { "fkst-dev:ready" }, { "fkst-dev:thinking" }, 18)
    handle.pr_edit_labels("owner/repo", 7, { "review" }, { "draft" }, 19)

    assert_argv_equal(
      calls[1].argv,
      { "gh", "api", "--paginate", "--slurp", "repos/owner/repo/issues?state=open&per_page=100" },
      "issue_list"
    )
    assert_argv_equal(
      calls[2].argv,
      { "gh", "api", "--paginate", "--slurp", "repos/owner/repo/pulls?state=open&per_page=100" },
      "pr_list"
    )
    assert_argv_equal(
      calls[3].argv,
      { "gh", "api", "--paginate", "--slurp", "repos/owner/repo/pulls?state=open&head=owner%3Afeature%2Fa&per_page=100&base=dev" },
      "pr_list_head"
    )
    assert_argv_equal(calls[4].argv, { "gh", "api", "repos/owner/repo/pulls/7" }, "pr_view")
    assert_argv_equal(
      calls[5].argv,
      { "gh", "pr", "create", "--repo", "owner/repo", "--head", "feature/a", "--base", "dev", "--title", "Fix title", "--body-file", "/tmp/body.md" },
      "pr_create"
    )
    assert_argv_equal(
      calls[6].argv,
      { "gh", "label", "list", "--repo", "owner/repo", "--limit", "1000", "--json", "name" },
      "label_list"
    )
    assert_argv_equal(
      calls[7].argv,
      { "gh", "label", "create", "fkst-dev:ready", "--repo", "owner/repo", "--color", "0E8A16" },
      "label_create"
    )
    assert_argv_equal(
      calls[8].argv,
      { "gh", "issue", "edit", "42", "--repo", "owner/repo", "--add-label", "fkst-dev:ready", "--remove-label", "fkst-dev:thinking" },
      "issue_edit_labels"
    )
    assert_argv_equal(
      calls[9].argv,
      { "gh", "pr", "edit", "7", "--repo", "owner/repo", "--add-label", "review", "--remove-label", "draft" },
      "pr_edit_labels"
    )
    for index, call in ipairs(calls) do
      assert(call.timeout == index + 10, "github method timeout mismatch for call " .. tostring(index))
      assert(call.cmd == nil, "github method must not pass cmd")
      assert(call.rate_pool == nil, "github method must not pass rate_pool")
    end
  end,

  test_github_comment_methods_build_argv = function()
    local calls = {}
    local handle = gh.new(function(opts)
      table.insert(calls, opts)
      return { stdout = "ok", stderr = "", exit_code = 0 }
    end)

    handle.issue_comments("owner/repo", 42, 31)
    handle.pr_comments("owner/repo", 7, 32)
    handle.issue_comment_create("owner/repo", 42, "/tmp/body.md", 33)
    handle.pr_comment_create("owner/repo", 7, "/tmp/body.md", 34)
    handle.comment_update("owner/repo", 123456, "/tmp/body.md", 35)
    handle.issue_comment("owner/repo", 42, "/tmp/body.md", 36)
    handle.pr_comment("owner/repo", 7, "/tmp/body.md", 37)

    assert_argv_equal(
      calls[1].argv,
      { "gh", "api", "--paginate", "--slurp", "repos/owner/repo/issues/42/comments?per_page=100" },
      "issue_comments"
    )
    assert_argv_equal(
      calls[2].argv,
      { "gh", "api", "--paginate", "--slurp", "repos/owner/repo/issues/7/comments?per_page=100" },
      "pr_comments"
    )
    assert_argv_equal(
      calls[3].argv,
      { "gh", "api", "--method", "POST", "repos/owner/repo/issues/42/comments", "--field", "body=@/tmp/body.md" },
      "issue_comment_create"
    )
    assert_argv_equal(
      calls[4].argv,
      { "gh", "api", "--method", "POST", "repos/owner/repo/issues/7/comments", "--field", "body=@/tmp/body.md" },
      "pr_comment_create"
    )
    assert_argv_equal(
      calls[5].argv,
      { "gh", "api", "--method", "PATCH", "repos/owner/repo/issues/comments/123456", "--field", "body=@/tmp/body.md" },
      "comment_update"
    )
    assert_argv_equal(
      calls[6].argv,
      { "gh", "issue", "comment", "42", "--repo", "owner/repo", "--body-file", "/tmp/body.md" },
      "issue_comment"
    )
    assert_argv_equal(
      calls[7].argv,
      { "gh", "pr", "comment", "7", "--repo", "owner/repo", "--body-file", "/tmp/body.md" },
      "pr_comment"
    )
    for index, call in ipairs(calls) do
      assert(call.argv[1] == "gh", "comment method must build gh argv for call " .. tostring(index))
      assert(call.timeout == index + 30, "comment method timeout mismatch for call " .. tostring(index))
      assert(call.cmd == nil, "comment method must not pass cmd")
      assert(call.rate_pool == nil, "comment method must not pass rate_pool")
    end
  end,

  test_git_methods_build_argv = function()
    local calls = {}
    local handle = git.new(function(opts)
      table.insert(calls, opts)
      return { stdout = "ok", stderr = "", exit_code = 0 }
    end)

    handle.push_branch("feature/a", 21)
    handle.show_ref_branch("feature/a", 22)
    handle.is_ancestor("abc123", "def456", 23)

    assert_argv_equal(calls[1].argv, { "git", "push", "-u", "origin", "feature/a" }, "push_branch")
    assert_argv_equal(calls[2].argv, { "git", "show-ref", "--verify", "refs/heads/feature/a" }, "show_ref_branch")
    assert_argv_equal(calls[3].argv, { "git", "merge-base", "--is-ancestor", "abc123", "def456" }, "is_ancestor")
    for index, call in ipairs(calls) do
      assert(call.timeout == index + 20, "git method timeout mismatch for call " .. tostring(index))
      assert(call.cmd == nil, "git method must not pass cmd")
      assert(call.rate_pool == nil, "git method must not pass rate_pool")
    end
  end,
}
