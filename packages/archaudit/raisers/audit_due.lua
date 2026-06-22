local core = require("core")

return {
  type = "cron",
  interval = core.audit_due_interval(),
  produces = "audit_due",
}
