local S = {}

function S.install(M)
  local shared = require("core.branches.shared").install(M)
  require("core.branches.git_mechanics").install(M, shared)
  require("core.branches.branch_train").install(M, shared)
  require("core.branches.pr_freshness").install(M, shared)
end

return S
