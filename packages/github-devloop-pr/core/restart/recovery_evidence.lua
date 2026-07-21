local recovery_evidence = require("devloop.recovery_evidence")

return recovery_evidence.inventory({
  "pr-open",
  "reviewing",
  "fixing",
  "review-meta",
  "merge-ready",
  "merging",
  "blocked",
})
