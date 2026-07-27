local devloop_base = require("devloop.base")
local t = fkst.test
local core = require("core")
local graph = require("testkit.graph")
local entity_read_mocks = require("tests.entity_read_mock_helpers")
local gh_argv = require("testkit.gh_argv_mock")
gh_argv.install(t, core)

local repo = "owner/repo"

local function mock_env(delivery_grants)
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
  t.mock_command(devloop_base.read_env_command("FKST_DEVLOOP_UPSTREAM_BRANCH"), {
    stdout = "dev",
    stderr = "",
    exit_code = 0,
  })
  t.mock_command(devloop_base.read_env_command("FKST_DEVLOOP_INTEGRATION_BRANCH"), {
    stdout = "",
    stderr = "",
    exit_code = 0,
  })
  t.mock_command(devloop_base.read_env_command("FKST_DEVLOOP_DELIVERY_GRANTS"), {
    stdout = delivery_grants or "",
    stderr = "",
    exit_code = 0,
  })
end

local function mock_empty_board(extra_pr_repo)
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
  if extra_pr_repo ~= nil then
    t.mock_command(core.gh_pr_list_observe_cmd(extra_pr_repo), {
      stdout = "[]\n",
      stderr = "",
      exit_code = 0,
    })
  end
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
    t.eq(trace.consumer_result.status, "accepted", tostring(trace.consumer_result.error or "liveness scan failed"))
    t.eq(#trace.raised, 0)
    graph.assert_covers(trace, {})
  end,

  test_liveness_scan_discovers_granted_implementation_pr_lane_alongside_host = function()
    local implementation_repo = "owner/implementation"
    local grants = '[{"lifecycle_repo":"owner/repo","lifecycle_issue":42,'
      .. '"implementation_repo":"' .. implementation_repo .. '",'
      .. '"implementation_branch":"fkst-hosted","implementation_root":"/runtime/implementation"}]'
    mock_env(grants)
    mock_empty_board(implementation_repo)

    local trace = t.fire_raiser("liveness_poll")

    t.eq(trace.consumer_result.status, "accepted", tostring(trace.consumer_result.error or "liveness scan failed"))
    local scanned_implementation = false
    for _, call in ipairs(t.command_calls()) do
      local rendered = tostring(call.rendered or "")
      if rendered:find("repos/owner/implementation/pulls", 1, true) ~= nil then
        scanned_implementation = true
      end
    end
    t.is_true(scanned_implementation)
  end,
}
