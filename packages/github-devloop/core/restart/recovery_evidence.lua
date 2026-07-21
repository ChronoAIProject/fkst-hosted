local recovery_evidence = require("devloop.recovery_evidence")

return recovery_evidence.inventory({
  "thinking",
  "dependency_wait",
  "ready",
  "implementing",
  "awaiting-pr",
  "impl-failed",
  "blocked",
})
