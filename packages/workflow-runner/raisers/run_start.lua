-- Poll for a dispatched run, at a cadence bounded well inside the schedule's own.
--
-- This was boot-once (`24h`, first fire jittered to ~30s), on the reasoning that
-- the pod only exists because a run issue woke the session, so the run is already
-- on GitHub by the time the raiser fires and polling would be redundant.
--
-- That reasoning holds only for a pod whose ONLY work is the run that woke it. A
-- session with any other open work — the default bundle watches five work-label
-- families — keeps its pod alive continuously, so the next run issue arrives at
-- an already-booted pod whose boot-once raiser fired hours ago. It would then
-- wait up to 24h while the control plane's watchdog released the schedule after
-- `FKST_CRON_MAX_RUNTIME_SECS` (3600s), recording a `timeout` for a run nothing
-- ever started. Only the first run after each boot could ever succeed.
--
-- 5m is chosen against the two limits that bracket it: the tightest cadence a
-- schedule may declare is `FKST_CRON_MIN_INTERVAL_SECS` (900s), so a 5-minute
-- detection latency cannot make a run miss its own next slot; and the watchdog
-- budget is 3600s, so it spends under 1% of it. The cost is one `gh issue list`
-- per tick, the same shape and cadence the dev loop's own sweep already runs in
-- this pod.
return {
  type = "cron",
  interval = "5m",
  produces = "scheduled_run_tick",
}
