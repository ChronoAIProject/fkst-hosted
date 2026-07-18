-- Reconcile department: one tick materializes the single authoring step (a codex
-- drafting run that opens a reviewable template PR) via the shared kernel. The spec
-- lives here (queue names); the handlers are the kernel's reconcile.handlers wired to
-- this adapter's seams from bindings.lua. No engine logic is copied into the package.
local saga = require("workflow.saga")
local reconcile = require("workflow.engine.reconcile")
local bindings = require("bindings")

local spec = {
  consumes = { "workflow_writer_materialization_tick" },
  produces = {
    "github-comment-effect.github_issue_comment_request",
  },
  stall_window = "10m",
}

return saga.department(spec, reconcile.handlers(bindings.seams()))
