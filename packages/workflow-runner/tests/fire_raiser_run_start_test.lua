local t = fkst.test

-- G-PRODUCER-LIVENESS: a raiser that nothing ever fires is a producer nobody can
-- prove works. This drives the real engine end to end — the cron raiser fires,
-- the event routes to `run_execute`, and the department accepts it. A raiser
-- whose queue name drifted from the department's `consumes` would otherwise fail
-- silently on every session, and silence is what the control plane reads as a
-- stalled engine.
--
-- No mocks, deliberately. The harness supplies no session environment, so the
-- env reads return nil, `claim_identity.read` refuses, and the department returns
-- having raised nothing. That IS the assertion worth making: the overwhelmingly
-- common case for this package is an ordinary session that composes it and has no
-- schedule at all, and such a session must boot, do nothing, and cost nothing.
--
-- It also pins the failure posture that makes that true. An earlier revision read
-- the environment with `propagate_exec_errors = true`, which turned an unreadable
-- value into a thrown error — every unscheduled session would have errored its
-- tick. This test fails if that regresses.

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
    t.eq(#trace.raised, 0)
  end,
}
