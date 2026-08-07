local t = fkst.test

return {
  test_run_start_polls_inside_the_tightest_declarable_cadence = function()
    -- Not boot-once. A session with other open work keeps its pod alive, so the
    -- next run issue arrives long after a boot-once raiser has fired and would
    -- wait up to 24h — past the control plane's 3600s watchdog, which records a
    -- `timeout` for a run nothing ever started.
    --
    -- The interval must stay strictly under FKST_CRON_MIN_INTERVAL_SECS (900s),
    -- the tightest cadence a schedule may declare, so detection latency can never
    -- make a run miss its own next slot.
    local raiser = require("raisers.run_start")
    t.eq(raiser.type, "cron")
    t.eq(raiser.interval, "5m")
    t.eq(raiser.produces, "scheduled_run_tick")
  end,
}
