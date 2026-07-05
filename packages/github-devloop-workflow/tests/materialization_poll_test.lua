local t = fkst.test

return {
  test_materialization_poll_raises_tick = function()
    local raiser = require("raisers.materialization_poll")
    t.eq(raiser.type, "cron")
    t.eq(raiser.interval, "5m")
    t.eq(raiser.produces, "workflow_materialization_tick")
  end,
}
