local devloop_base = require("devloop.base")
local t = fkst.test
local core = require("core")
local graph = require("testkit.graph")
local entity_read_mocks = require("tests.entity_read_mock_helpers")
local gh_argv = require("testkit.gh_argv_mock")
gh_argv.install(t, core)

local repo = "owner/repo"

local function mock_env()
  t.mock_command(devloop_base.read_env_command("FKST_GITHUB_REPO"), {
    stdout = repo,
    stderr = "",
    exit_code = 0,
  })
  t.mock_command(devloop_base.read_env_command("FKST_GITHUB_BOT_LOGIN"), {
    stdout = "fkst-test-bot",
    stderr = "",
    exit_code = 0,
  })
end

local function mock_empty_board()
  t.mock_command(core.gh_issue_list_observe_cmd(repo), {
    stdout = "[]\n",
    stderr = "",
    exit_code = 0,
  })
  t.mock_command(core.gh_pr_list_observe_cmd(repo), {
    stdout = "[]\n",
    stderr = "",
    exit_code = 0,
  })
  entity_read_mocks.mock_issue_read_forms(t, {
    repo = repo,
    number = 0,
    comments = {},
    labels = {},
    state = "OPEN",
  })
end

return {
  test_fire_raiser_liveness_poll_routes_real_tick_to_scan = function()
    mock_env()
    mock_empty_board()
    local trace = t.fire_raiser("liveness_poll")
    t.eq(trace.source_ref.kind, "cron")
    t.eq(trace.source_payload.raiser, "github-devloop-pr.liveness_poll")
    t.eq(trace.routed_to[1], "github-devloop-pr.liveness_scan")
    t.eq(trace.consumer_result.status, "accepted")
    t.eq(#trace.raised, 0)
    graph.assert_covers(trace, {})
  end,
}
