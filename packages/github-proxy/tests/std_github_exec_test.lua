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
}
