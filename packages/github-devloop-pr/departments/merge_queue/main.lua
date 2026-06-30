local core, saga = require("core"), require("workflow.saga")
local merge_executor = require("core.merge_executor")
local queue = require("devloop.queue")

local spec = {
  consumes = { "devloop_merge_queue_tick" },
  produces = {
    "github-proxy.github_issue_label_request",
    "github-proxy.github_pr_comment_request",
    "devloop_fix_reconcile",
    "github-devloop-decompose.devloop_decompose",
    "devloop_merge_queue_tick",
  },
  fanout = { "devloop_merge_queue_tick" },
  stall_window = "2m",
  retry = { max_attempts = 12, base = "5s", cap = "30s" },
}

return saga.department(spec, { done = function() return false end, act = function(event)
  queue.dispatch_consumed_queue("merge_queue", spec, event, {
    devloop_merge_queue_tick = merge_executor.process_merge_queue_tick,
  }, "github-devloop-pr")
end, wrap = core.wrap_pipeline_failure, name = "merge_queue" })
