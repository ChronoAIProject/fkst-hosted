local gh = require("std.github")

return {
  test_exec_classifies_rate_limit = function()
    local handle = gh.new(function(_opts)
      return { stdout = "", stderr = "API rate limit exceeded for user", exit_code = 1 }
    end)
    local ok, err = pcall(function()
      return handle._exec("gh api x", 10, "ctx")
    end)
    assert(ok == false)
    assert(err.class == "gh-rate-limited", "rate-limit stderr must classify as gh-rate-limited")
    assert(err.retryable == true)
  end,

  test_exec_classifies_generic_failure = function()
    local handle = gh.new(function(_opts)
      return { stdout = "", stderr = "fatal: not found", exit_code = 1 }
    end)
    local ok, err = pcall(function()
      return handle._exec("gh api y", 10, "ctx")
    end)
    assert(ok == false)
    assert(err.class == "gh-command-failed")
  end,

  test_exec_returns_result_on_success = function()
    local handle = gh.new(function(_opts)
      return { stdout = "ok", stderr = "", exit_code = 0 }
    end)
    local out = handle._exec("gh api z", 10, "ctx")
    assert(out.stdout == "ok")
  end,
}
