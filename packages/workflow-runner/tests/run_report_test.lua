local t = fkst.test

--- Capture what a department raises, without an engine.
local function capture(department, event)
  local raised = {}
  local previous = _G.raise
  _G.raise = function(queue, payload)
    raised[#raised + 1] = { queue = queue, payload = payload }
  end
  local ok, err = pcall(department.pipeline, event)
  _G.raise = previous
  return raised, ok, err
end

local function result_event()
  return {
    queue = "scheduled_run_result",
    payload = {
      repo = "acme/site",
      schedule_issue = 123,
      run_issue = 456,
      record = {
        slot = "2026-08-05T01:00:00Z",
        manual = false,
        status = "ok",
        started = "2026-08-05T01:00:00Z",
        ended = "2026-08-05T01:12:00Z",
        issue = 456,
        steps = { { index = 1, id = "scrape", status = "ok", duration_s = 41 } },
      },
    },
  }
end

return {
  test_the_record_is_posted_on_the_DEFINITION_issue_not_the_run_issue = function()
    -- The definition issue is where the clock reads its history, where the API
    -- projects the run list, and where an operator looks. A record on the run
    -- issue would be invisible to all three.
    local department = require("departments.run_report.main")
    local raised = capture(department, result_event())
    t.eq(#raised, 1)
    t.eq(raised[1].queue, "github_issue_comment_request")
    t.eq(raised[1].payload.issue_number, 123)
    t.eq(raised[1].payload.repo, "acme/site")
  end,

  test_the_comment_carries_the_run_marker = function()
    local department = require("departments.run_report.main")
    local raised = capture(department, result_event())
    t.is_true(raised[1].payload.body:find("fkst%-cron%-run:v1") ~= nil)
    t.is_true(raised[1].payload.body:find("✅ Scheduled run succeeded", 1, true) ~= nil)
  end,

  test_the_dedup_key_is_scoped_to_the_slot = function()
    -- Slot-scoped rather than run-scoped, so a redelivered result cannot post a
    -- second record for a slot the control plane has already completed.
    local department = require("departments.run_report.main")
    local raised = capture(department, result_event())
    t.eq(raised[1].payload.dedup_key, "workflow-runner/run/123/2026-08-05T01:00:00Z")
  end,

  test_it_never_emits_a_label_request = function()
    -- The control plane is the single writer of every fkst-cron-* label; that is
    -- what makes the overlap rule and the watchdog trustworthy. A label written
    -- from here would race the reconciler for state it does not own.
    local department = require("departments.run_report.main")
    local raised = capture(department, result_event())
    for _, entry in ipairs(raised) do
      t.is_true(
        tostring(entry.queue):find("label", 1, true) == nil,
        "workflow-runner must never request a label change"
      )
    end
    for _, produced in ipairs(department.spec.produces) do
      t.is_true(tostring(produced):find("label", 1, true) == nil)
    end
  end,

  test_a_malformed_result_fails_loudly = function()
    local department = require("departments.run_report.main")
    local _, ok = capture(department, { queue = "scheduled_run_result", payload = {} })
    t.eq(ok, false)
  end,

  test_an_unknown_queue_is_refused = function()
    local department = require("departments.run_report.main")
    local _, ok = capture(department, { queue = "something_else", payload = {} })
    t.eq(ok, false)
  end,
}
