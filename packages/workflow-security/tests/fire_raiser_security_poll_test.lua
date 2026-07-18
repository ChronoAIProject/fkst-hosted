-- Producer-liveness: the security_poll cron raiser must route to the
-- security_select department. With no discoverable review issues (the GitHub port
-- degrades to empty), the tick is accepted and produces no downstream request --
-- the honest "nothing overdue" trace. This asserts the four trace fields the
-- producer-liveness ratchet requires: consumer_result / source_payload / raised /
-- routed_to.
local helper = require("tests.fire_raiser_helpers")
local t = fkst.test

return {
  test_fire_raiser_security_poll_routes_to_security_select = function()
    local root = helper.setup_workspace("route", helper.fire_raiser_child([[
  test_security_poll_routes = function()
    mock_env()

    local trace = t.fire_raiser("security_poll")
    t.eq(trace.source_payload.raiser, "workflow-security.security_poll")
    t.eq(trace.routed_to[1], "workflow-security.security_select")
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
