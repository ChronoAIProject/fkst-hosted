local t = fkst.test

-- G-PRODUCER-LIVENESS: a raiser that nothing ever fires is a producer nobody can
-- prove works. This drives the real engine end to end — the cron raiser fires,
-- the event routes to `run_execute`, and the department accepts it.

return {
  test_fire_raiser_run_start_routes_a_real_tick_to_run_execute = function()
    local trace = t.fire_raiser("run_start")

    t.eq(trace.source_ref.kind, "cron")
    t.eq(trace.source_payload.raiser, "run_start")
    t.eq(trace.routed_to[1], "run_execute")
    if trace.consumer_result.status ~= "accepted" then
      error(trace.consumer_result.message or "fire_raiser consumer failed")
    end
    t.eq(trace.consumer_result.status, "accepted")

    -- The harness has no verified repo scope (no FKST_GITHUB_REPO), so the tick
    -- routes, the department decides it has no repository to look for a run
    -- issue in, and raises nothing. That is the fleet-safety property in
    -- miniature: a session this package cannot scope costs a run, never an
    -- error — an ordinary session composing it must boot and do nothing.
    t.eq(#trace.raised, 0)
  end,
}
