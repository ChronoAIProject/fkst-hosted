local t = fkst.test

return {
  test_run_start_is_a_boot_once_raiser = function()
    -- `cron_first_fire_jitter` bounds the first fire by min(interval, 30s), so a
    -- 24h interval fires within ~30s of pod start and never again before the pod
    -- idles down. The pod only exists because a run issue woke the session, so
    -- polling would be both redundant and a standing cost.
    local raiser = require("raisers.run_start")
    t.eq(raiser.type, "cron")
    t.eq(raiser.interval, "24h")
    t.eq(raiser.produces, "scheduled_run_tick")
  end,
}
