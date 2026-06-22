local raiser = require("raisers.audit_poll")
local core = require("core")
local t = fkst.test

return {
  test_audit_poll_raises_audit_tick_every_thirty_minutes = function()
    t.eq(raiser.type, "cron")
    t.eq(raiser.interval, core.audit_poll_interval())
    t.eq(raiser.produces, "archaudit_tick")
  end,
}
