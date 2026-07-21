local t = fkst.test

local startup = require("raisers.liveness_startup")
local periodic = require("raisers.liveness_poll")
local previous_pipeline = pipeline
local liveness_scan = require("departments.liveness_scan.main")
pipeline = previous_pipeline

return {
  test_startup_and_periodic_raisers_share_the_liveness_seam = function()
    t.eq(startup.type, "file_watch")
    t.eq(startup.glob, ".git/HEAD")
    t.eq(startup.produces, "devloop_liveness_tick")

    t.eq(periodic.type, "cron")
    t.eq(periodic.interval, "5m")
    t.eq(periodic.produces, startup.produces)
    t.eq(liveness_scan.spec.consumes[1], startup.produces)
    t.eq(liveness_scan.spec.published_seam[1], startup.produces)
  end,

  test_liveness_scan_retries_transient_delivery_failures_before_the_cron = function()
    t.eq(liveness_scan.spec.retry.max_attempts, 6)
    t.eq(liveness_scan.spec.retry.base, "5s")
    t.eq(liveness_scan.spec.retry.cap, "60s")
  end,
}
