local t = fkst.test

return {
  test_idle_poll_cron_shape = function()
    local raiser = require("raisers.idle_poll")
    t.eq(raiser.type, "cron")
    t.eq(raiser.interval, "30m")
    t.eq(raiser.produces, "idle_tick")
  end,
}
