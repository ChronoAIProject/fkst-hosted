local S = {}

function S.install(M, resolved)
  local shared = require("std.devloop_liveness.shared").install(M, resolved)
  require("std.devloop_liveness.contract").install(M, shared)
  require("std.devloop_liveness.signal").install(M, shared)
  require("std.devloop_liveness.timeout").install(M, shared)
end

return S
