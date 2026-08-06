-- Boot-once.
--
-- `cron_first_fire_jitter` bounds a cron raiser's first fire by
-- min(interval, 30s), so a 24h interval fires within ~30 seconds of pod start and
-- never again before the pod idles down. That is exactly the shape this package
-- needs: the pod only exists because a run issue woke the session, so the run's
-- identity is already on GitHub and there is nothing to poll for. A polling
-- raiser would be both redundant and a standing cost.
return {
  type = "cron",
  interval = "24h",
  produces = "scheduled_run_tick",
}
