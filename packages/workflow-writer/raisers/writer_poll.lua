-- workflow-writer owns its work: this cron tick drives the workflow_writer_select
-- department to poll fkst-workflow request issues and advance the authoring flow. It is
-- the adapter's OWN trigger path -- it NEVER consumes the dev intake candidate seam.
return {
  type = "cron",
  interval = "30m",
  produces = "workflow_writer_tick",
}
