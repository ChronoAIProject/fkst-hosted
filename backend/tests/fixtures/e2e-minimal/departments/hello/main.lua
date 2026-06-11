-- e2e-minimal: the smallest engine-runnable department (issue #17 spike,
-- docs/spikes/issue-17-engine-host-contract.md "Minimal package").
--
-- Shape: `M.spec` + a GLOBAL `pipeline(event)`. There is NO `name` field in
-- the spec — identity is the directory name (`hello`); the PACKAGE identity
-- is the `packages._id` (`e2e-minimal`) and is independent of this name.
--
-- Deliberately raiser-less: conformance passes (with a no-producer warning)
-- and `supervise` starts, emits both ready markers ("event runtime running"
-- and "consumer started"), then idles — exactly what the happy-path e2e
-- needs to observe `running` without any external inputs.
local M = {}

M.spec = {
  consumes = { "tick" },
  stall_window = "30s",
}

function pipeline(event)
  log.info("hello received event on queue: " .. tostring(event.queue))
  return event
end

return M
