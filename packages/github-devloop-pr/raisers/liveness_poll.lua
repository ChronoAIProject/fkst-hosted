local config = require("devloop.config")

return {
  type = "cron",
  interval = config.liveness_poll_interval(),
  produces = "devloop_liveness_tick",
}
