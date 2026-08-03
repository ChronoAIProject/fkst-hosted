-- Producer liveness (G-PRODUCER-LIVENESS): the health_poll cron raiser must route a
-- real tick to the health_report department through the engine's own router. This is
-- the only test that proves the declared raiser is actually wired -- a raiser whose
-- queue name drifts from the department's `consumes` would otherwise fail silently on
-- every session, and silence is exactly what the control plane reads as a stalled
-- engine.
--
-- The four trace fields the ratchet requires are all asserted: consumer_result,
-- source_payload, raised, routed_to.
local graph = require("testkit.graph")
local t = fkst.test

return {
  test_fire_raiser_health_poll_routes_a_real_tick_to_health_report = function()
    local trace = t.fire_raiser("health_poll")

    t.eq(trace.source_ref.kind, "cron")
    t.eq(trace.source_payload.raiser, "health_poll")
    t.eq(trace.routed_to[1], "health_report")
    if trace.consumer_result.status ~= "accepted" then
      error(trace.consumer_result.message or "fire_raiser consumer failed")
    end
    t.eq(trace.consumer_result.status, "accepted")

    -- No FKST_SESSION_ID exists in the test harness, so the tick routes, decides it
    -- has no session identity to report for, logs that, and raises nothing. That is
    -- the fleet-safety property in miniature: a session the reporter cannot identify
    -- costs a report, never an error.
    t.eq(#trace.raised, 0)

    -- fkst-health is self-contained: its cron tick, its queue, its department. No
    -- cross-package edge exists for the integration-coverage ratchet to require.
    graph.assert_covers(trace, {})
  end,
}
