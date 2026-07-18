-- Producer-liveness: the writer_poll cron raiser must route to the
-- workflow_writer_select department. With no discoverable request issues (the GitHub
-- port degrades to empty), the tick is accepted and produces no downstream request --
-- the honest "nothing overdue" trace. This asserts the four trace fields the
-- producer-liveness ratchet requires: consumer_result / source_payload / raised /
-- routed_to.
local helper = require("tests.fire_raiser_helpers")
local t = fkst.test

return {
  test_fire_raiser_writer_poll_routes_to_writer_select = function()
    local root = helper.setup_workspace("route", helper.fire_raiser_child([[
  test_writer_poll_routes = function()
    mock_env()

    local trace = t.fire_raiser("writer_poll")
    t.eq(trace.source_payload.raiser, "workflow-writer.writer_poll")
    t.eq(trace.routed_to[1], "workflow-writer.workflow_writer_select")
    if trace.consumer_result.status ~= "accepted" then
      error(trace.consumer_result.message or "fire_raiser consumer failed")
    end
    t.eq(trace.consumer_result.status, "accepted")
    t.eq(#trace.raised, 0)
  end,
]]))
    local output = helper.run_child(root)
    t.is_true(output:find("1 passed, 0 failed", 1, true) ~= nil, output)
  end,
}
