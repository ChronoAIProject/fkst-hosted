-- workflow-security owns its work: this cron tick drives the security_select
-- department to poll fkst-security review issues and advance the pipeline. It is
-- the adapter's OWN trigger path -- it NEVER consumes the dev intake candidate seam.
return {
  type = "cron",
  interval = "30m",
  produces = "workflow_security_tick",
}
