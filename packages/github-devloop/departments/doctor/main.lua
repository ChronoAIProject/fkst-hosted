local core = require("core")

local M = {}

M.spec = {
  consumes = { "devloop_doctor_tick" },
  produces = {},
  retry = false,
  stall_window = "2m",
}

function pipeline(_event)
  print(core.saga_doctor_run())
end

pipeline = core.wrap_pipeline_failure("doctor", pipeline)

return M
