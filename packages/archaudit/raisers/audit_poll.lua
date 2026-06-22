local core = require("core")

return {
  type = "cron",
  interval = core.audit_poll_interval(),
  produces = "archaudit_tick",
}
