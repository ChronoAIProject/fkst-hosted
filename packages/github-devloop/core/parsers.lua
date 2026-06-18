local S = {}

function S.install(M)
  local shared = require("core.parsers.shared").install(M)
  require("core.parsers.issue").install(M, shared)
  require("core.parsers.pr").install(M, shared)
  require("core.parsers.misc").install(M, shared)
end

return S
