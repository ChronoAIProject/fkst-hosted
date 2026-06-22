local S = {}

function S.install(M, resolved)
  local shared = require("devloop.liveness.shared").install(M, resolved)
  require("devloop.liveness.contract").install(M, shared)
  require("devloop.liveness.signal").install(M, shared)
  require("devloop.liveness.timeout").install(M, shared)
end

return S
