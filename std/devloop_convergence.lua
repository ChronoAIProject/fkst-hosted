local S = {}

function S.install(M)
  local shared = require("std.devloop_convergence.shared")
  shared.install(M)
  require("std.devloop_convergence.rounds").install(M, shared)
  require("std.devloop_convergence.reconcile").install(M, shared)
  require("std.devloop_convergence.attempts").install(M, shared)
end

return S
