local S = {}

function S.install(M)
  local shared = require("std.devloop_parsers.shared").install(M)
  require("std.devloop_parsers.issue").install(M, shared)
  require("std.devloop_parsers.pr").install(M, shared)
  require("std.devloop_parsers.misc").install(M, shared)
end

return S
