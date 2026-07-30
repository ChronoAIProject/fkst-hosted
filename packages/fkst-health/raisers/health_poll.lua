local health = require("health")

-- WHY 10 minutes and not idle-detector's 30.
--
-- A session pod is reaped once it has no pending work, after session_idle_grace_secs
-- (default 300s) with pod_min_lifetime_secs (default 120s) as the floor. A session
-- whose work takes ten minutes therefore lives ~15 minutes end to end, so a 30-minute
-- tick would fire ZERO times for exactly the short sessions users watch most closely.
--
-- Ticks stopping when the pod is reaped is correct, not a gap: a session with nothing
-- left to do has nothing to report, and the control plane distinguishes "no live
-- runtime" from "stale reports" on its side.
--
-- The interval is derived from health.expected_interval_seconds rather than written
-- twice: the same number is stamped into every report as expected_interval_secs, and
-- the control plane's staleness watchdog reads it from there. Two literals could
-- drift apart and make the watchdog misjudge every session.
return {
  type = "cron",
  interval = tostring(math.floor(health.expected_interval_seconds / 60)) .. "m",
  produces = "health_tick",
}
