local S = {}

function S.install(M)
  local shared = require("devloop.git_mechanics").helpers(M)
  require("core.branches.branch_train").install(M, shared)
  require("core.branches.pr_freshness").install(M, shared)
end

return S
