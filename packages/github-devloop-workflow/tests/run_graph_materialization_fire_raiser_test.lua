local devloop_base = require("devloop.base")
local t = fkst.test
local core = require("core")
local graph = require("testkit.graph")
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

local function mock_empty_origin_list()
  t.mock_command(core.gh_issue_list_observe_cmd(repo), {
    stdout = "[]\n",
    stderr = "",
    exit_code = 0,
  })
end

return {
  test_fire_raiser_materialization_poll_routes_real_tick_to_materializer = function()
    mock_env()
    mock_empty_origin_list()
    local trace = t.fire_raiser("materialization_poll")
    t.eq(trace.source_ref.kind, "cron")
    t.eq(trace.source_payload.raiser, "github-devloop-workflow.materialization_poll")
    t.eq(trace.routed_to[1], "github-devloop-workflow.workflow_materialize_next")
    t.eq(trace.consumer_result.status, "accepted")
    t.eq(#trace.raised, 0)
    graph.assert_covers(trace, {})
  end,
}
