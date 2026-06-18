local S = {}

function S.install(M)
  local shared = require("core.liveness.shared").install(M)
  require("core.liveness.contract").install(M, shared)
  require("core.liveness.signal").install(M, shared)
  require("core.liveness.timeout").install(M, shared)
end

return S
