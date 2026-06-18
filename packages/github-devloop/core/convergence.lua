local S = {}

function S.install(M)
  local shared = require("core.convergence.shared")
  shared.install(M)
  require("core.convergence.rounds").install(M, shared)
  require("core.convergence.reconcile").install(M, shared)
  require("core.convergence.attempts").install(M, shared)
end

return S
