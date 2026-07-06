local dead_letter = require("workflow.dead_letter")
local saga = require("workflow.saga")

local spec = {
  consumes = { "dead_letter" },
  produces = {},
  stall_window = "2m",
}

return saga.department(spec, dead_letter.handlers({
  package = "github-devloop-workflow",
}))
