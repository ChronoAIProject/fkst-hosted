local runner = require("runner")
local saga = require("workflow.saga")

-- Report one run onto its DEFINITION issue.
--
-- The definition issue, not the run issue, is deliberate: the definition is where
-- the control plane's clock reads its history, where the API projects the run
-- list, and where an operator looks to see whether a schedule is healthy. A
-- record posted on the run issue would be invisible to all three.
--
-- This department NEVER emits a label request. The control plane is the single
-- writer of every `fkst-cron-*` label — that is what makes the overlap rule and
-- the watchdog trustworthy — and it drops `fkst-cron-running` when it sees the
-- terminal record this comment carries. A label written from here would race the
-- reconciler for state it does not own.
local spec = {
  consumes = { "scheduled_run_result" },
  -- FULLY QUALIFIED. The seam is published by github-proxy; a bare name is
  -- namespaced to THIS package, so the request would land on
  -- `workflow-runner.github_issue_comment_request`, which nothing consumes.
  -- The engine says so at every boot ("produced by ... but has no consumer"),
  -- and the effect is that a finished run posts no record at all: the control
  -- plane never sees completion and the schedule sits until its watchdog
  -- expires, recording every run as `timeout` whatever it really did.
  produces = { "github-proxy.github_issue_comment_request" },
  stall_window = "30s",
}

local function is_result(event)
  local queue = tostring(event and event.queue or "")
  return queue == "scheduled_run_result" or queue == "workflow-runner.scheduled_run_result"
end

--- The dedup key: one record per (definition, slot).
---
--- Slot-scoped rather than run-scoped so a redelivered result cannot post a
--- second record for a slot the control plane has already completed.
local function dedup_key(payload)
  return ("workflow-runner/run/%s/%s"):format(
    tostring(payload.schedule_issue),
    tostring(payload.record and payload.record.slot)
  )
end

return saga.department(spec, {
  done = function(event)
    if not is_result(event) then
      error("workflow-runner: unknown-queue: " .. tostring(event and event.queue), 0)
    end
    return false
  end,
  act = function(event)
    local payload = (type(event) == "table" and event.payload) or {}
    local record = payload.record
    if type(record) ~= "table" or payload.schedule_issue == nil then
      error("workflow-runner: malformed-run-result: missing record or schedule issue", 0)
    end
    raise("github-proxy.github_issue_comment_request", {
      schema = "github-proxy.v1",
      repo = payload.repo,
      issue_number = payload.schedule_issue,
      body = runner.render_report(record),
      dedup_key = dedup_key(payload),
      source_ref = {
        kind = "external",
        ref = ("%s#issue/%s"):format(tostring(payload.repo), tostring(payload.schedule_issue)),
      },
    })
  end,
  name = "run_report",
})
