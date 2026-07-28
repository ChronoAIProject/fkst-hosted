local h = require("tests.proxy_integration_helpers")
local t = h.t

local issue = h.poll_issue_json(60, "2026-06-03T01:02:00Z")

local function mock_poll()
  h.mock_repo_env()
  h.mock_poll_label_prefix_env("adapter-")
  h.mock_proxy_replay_budget_env("10")
  h.mock_issue_list(h.poll_issue_list_from({ issue }))
  h.mock_pr_list("[]\n")
end

local function run_poll(run_opts, token)
  return t.run_department("departments/github_poll/main.lua", {
    queue = "github_poll_tick",
    payload = {},
    ts = token,
  }, run_opts)
end

return {
  test_assignee_mode_emits_observed_issue_replay = function()
    local run_opts = h.opts("assignee-mode-observation", {
      FKST_GITHUB_CLAIM_MODE = "assignee",
    })

    mock_poll()
    local first = run_poll(run_opts, "assignee-poll-1")
    t.eq(first.exit_code, 0)
    t.eq(#h.changed_raises(first.raises), 1)

    mock_poll()
    local second = run_poll(run_opts, "assignee-poll-2")
    t.eq(second.exit_code, 0)
    t.eq(#h.changed_raises(second.raises), 0)
    t.eq(#h.observed_issue_raises(second.raises), 1)
  end,

  test_label_mode_suppresses_observed_issue_replay = function()
    local run_opts = h.opts("label-mode-observation", {
      FKST_GITHUB_CLAIM_MODE = "label",
    })

    mock_poll()
    local first = run_poll(run_opts, "label-poll-1")
    t.eq(first.exit_code, 0)
    t.eq(#h.changed_raises(first.raises), 1)

    mock_poll()
    local second = run_poll(run_opts, "label-poll-2")
    t.eq(second.exit_code, 0)
    t.eq(#h.changed_raises(second.raises), 0)
    t.eq(#h.observed_issue_raises(second.raises), 0)
    t.eq(#second.raises, 0)
  end,
}
