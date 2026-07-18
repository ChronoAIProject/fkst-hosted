-- workflow-security owns its work: this cron tick drives the security_select
-- department to poll fkst-security review issues and advance the pipeline. It is
-- the adapter's OWN trigger path -- it NEVER consumes the dev intake candidate seam.
--
-- This poll is the pipeline's PRIMARY advance driver: each tick materializes at
-- most one frontier step, so the interval bounds the pace of a multi-step review
-- (unlike github-devloop, which is additionally event-driven and treats its 5m
-- poll as a fallback). 1m keeps an interactive review responsive without
-- approaching GitHub API rate limits.
return {
  type = "cron",
  interval = "1m",
  produces = "workflow_security_tick",
}
