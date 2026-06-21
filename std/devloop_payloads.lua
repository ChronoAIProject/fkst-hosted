local S = {}

local shared = require("std.devloop_payloads.shared")
local board = require("std.devloop_payloads.board")
local predicates = require("std.devloop_payloads.predicates")
local builders = require("std.devloop_payloads.builders")

function S.install(M)
  local context = shared.install(M)
  board.install(M, context)
  predicates.install(M, context)
  builders.install(M, context)
end

return S
