local h = require("tests.proxy_integration_helpers")
local t = h.t

local function mock_poll(issues)
  h.mock_repo_env()
  h.mock_poll_label_prefix_env("adapter-")
  h.mock_proxy_replay_budget_env("10")
  h.mock_issue_list(issues)
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
  test_level_replays_only_exact_creator_route_when_configured = function()
    local run_opts = h.opts("creator-routed-level-replay", {
      FKST_GITHUB_PROXY_REPLAY_BUDGET = "10",
      FKST_SESSION_CREATOR = "Creator-Login",
    })
    local issues = h.poll_issue_list_from({
      '{"number":50,"title":"Routed","html_url":"https://github.example/owner/x/issues/50","updated_at":"2026-06-03T01:01:00Z","state":"open","author":{"login":"trusted-human"},"labels":[{"name":"bug"}],"assignees":[{"login":"creator-login"}]}',
      '{"number":51,"title":"Unassigned","html_url":"https://github.example/owner/x/issues/51","updated_at":"2026-06-03T01:02:00Z","state":"open","author":{"login":"trusted-human"},"labels":[{"name":"bug"}],"assignees":[]}',
      '{"number":52,"title":"Foreign","html_url":"https://github.example/owner/x/issues/52","updated_at":"2026-06-03T01:03:00Z","state":"open","author":{"login":"trusted-human"},"labels":[{"name":"bug"}],"assignees":[{"login":"other-login"}]}',
      '{"number":53,"title":"Ambiguous","html_url":"https://github.example/owner/x/issues/53","updated_at":"2026-06-03T01:04:00Z","state":"open","author":{"login":"trusted-human"},"labels":[{"name":"bug"}],"assignees":[{"login":"creator-login"},{"login":"other-login"}]}',
    })

    mock_poll(issues)
    local first = run_poll(run_opts, "creator-poll-1")
    t.eq(first.exit_code, 0)
    t.eq(#h.changed_raises(first.raises), 4)
    t.eq(h.find_entity_raise(first.raises, "issue", 50).payload.dedup_key,
      "owner/x#issue#50@2026-06-03T01:01:00Z/poll/creator-poll-1")

    mock_poll(issues)
    local second = run_poll(run_opts, "creator-poll-2")
    t.eq(second.exit_code, 0)
    t.eq(#h.changed_raises(second.raises), 1)
    t.eq(h.changed_raises(second.raises)[1].payload.number, 50)
    t.eq(h.changed_raises(second.raises)[1].payload.dedup_key,
      "owner/x#issue#50@2026-06-03T01:01:00Z/poll/creator-poll-2")
    t.eq(#h.observed_issue_raises(second.raises), 3)
  end,
}
