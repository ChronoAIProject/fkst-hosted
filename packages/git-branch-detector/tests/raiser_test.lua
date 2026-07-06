local t = fkst.test

return {
  test_git_ref_poll_cron_shape = function()
    local raiser = require("raisers.git_ref_poll")
    t.eq(raiser.type, "cron")
    t.eq(raiser.interval, "5m")
    t.eq(raiser.produces, "git_ref_poll_tick")
  end,
}
