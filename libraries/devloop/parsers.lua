local S = {}

function S.install(M)
  local shared = require("devloop.parsers.shared").install(M)
  require("devloop.parsers.issue").install(M, shared)
  require("devloop.parsers.pr").install(M, shared)
  require("devloop.parsers.misc").install(M, shared)
end

return S
