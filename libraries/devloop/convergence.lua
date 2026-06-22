local S = {}

function S.install(M)
  local shared = require("devloop.convergence.shared")
  shared.install(M)
  require("devloop.convergence.rounds").install(M, shared)
  require("devloop.convergence.reconcile").install(M, shared)
  require("devloop.convergence.attempts").install(M, shared)
end

return S
