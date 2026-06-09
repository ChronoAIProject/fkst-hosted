local h = require("tests.devloop_helpers")
local t = h.t
local core = h.core
local opts = h.opts
local issue = h.issue
local run_open_pr = h.run_open_pr
local mock_issue_open_pr = h.mock_issue_open_pr
local count_calls = h.count_calls

local function assert_clean_open_pr_skip(result)
  t.eq(result.exit_code, 0)
  t.eq(#result.raises, 0)
  t.eq(count_calls("show-ref --verify --quiet"), 0)
  t.eq(count_calls("rev-parse --verify"), 0)
  t.eq(count_calls("git -C"), 0)
end

return {
  test_open_pr_skips_entity_changed_with_no_state_marker = function()
    mock_issue_open_pr({ "fkst-dev:enabled" }, {})

    local result = run_open_pr(issue({ labels = { "fkst-dev:enabled" } }), opts("open-pr-no-state-marker"))

    assert_clean_open_pr_skip(result)
  end,

  test_open_pr_skips_entity_changed_for_thinking_issue = function()
    mock_issue_open_pr({ "fkst-dev:thinking" }, {
      core.state_marker("github-devloop/issue/owner/repo/42", "thinking", "2026-06-02T00-00-00Z"),
    })

    local result = run_open_pr(issue({ labels = { "fkst-dev:thinking" } }), opts("open-pr-thinking"))

    assert_clean_open_pr_skip(result)
  end,

  test_open_pr_skips_entity_changed_for_ready_issue = function()
    mock_issue_open_pr({ "fkst-dev:ready" }, {
      core.state_marker("github-devloop/issue/owner/repo/42", "ready", "2026-06-02T00-00-00Z"),
    })

    local result = run_open_pr(issue({ labels = { "fkst-dev:ready" } }), opts("open-pr-ready"))

    assert_clean_open_pr_skip(result)
  end,

  test_open_pr_retries_when_implementing_fact_marker_missing = function()
    local impl_version = "ready/consensus-github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z"
    mock_issue_open_pr({ "fkst-dev:implementing" }, {
      core.state_marker("github-devloop/issue/owner/repo/42", "implementing", impl_version),
    })

    local result = run_open_pr(issue({ labels = { "fkst-dev:implementing" } }), opts("open-pr-missing-implementing-fact"))

    t.eq(result.exit_code, 1)
    t.eq(#result.raises, 0)
    t.eq(count_calls("show-ref --verify --quiet"), 0)
    t.eq(count_calls("rev-parse --verify"), 0)
  end,
}
