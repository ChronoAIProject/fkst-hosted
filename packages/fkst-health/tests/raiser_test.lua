local health = require("health")
local t = fkst.test

return {
  test_health_poll_cron_shape = function()
    local raiser = require("raisers.health_poll")
    t.eq(raiser.type, "cron")
    t.eq(raiser.interval, "10m")
    t.eq(raiser.produces, "health_tick")
  end,

  -- The report's expected_interval_secs and the raiser's cron interval are the same
  -- number by construction. If they ever drift the control plane's staleness
  -- watchdog judges every session against the wrong cadence, so the tie is asserted.
  test_raiser_interval_is_derived_from_the_declared_report_cadence = function()
    local raiser = require("raisers.health_poll")
    t.eq(health.expected_interval_seconds, 600)
    t.eq(raiser.interval, tostring(math.floor(health.expected_interval_seconds / 60)) .. "m")
  end,
}
