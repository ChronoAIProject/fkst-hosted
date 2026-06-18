local core = require("core")
local saga = require("std.saga")

local spec = {
  consumes = { "devloop_branch_tick" },
  produces = { "github-proxy.github_issue_create_request" },
  fanout = { "devloop_branch_tick" },
  stall_window = "5m",
}

local function done(_event)
  return false
end

local function act(event)
  core.log_entry("ratchet_slice_scan", event, "ratchet-slicer", event and event.queue or "")
  local repo = core.configured_repo()
  if repo == nil or repo == "" or core.safe_repo(repo) ~= repo then
    core.log_cas_decision("ratchet_slice_scan", "ratchet-slicer", { state = nil, version = nil }, "tick", "slice", "skip-invalid-repo", "FKST_GITHUB_REPO is missing or invalid")
    return
  end
  core.reconcile_ratchet_slices(repo)
end

return saga.department(spec, {
  done = done,
  act = act,
  wrap = core.wrap_pipeline_failure,
  name = "ratchet_slice_scan",
})
