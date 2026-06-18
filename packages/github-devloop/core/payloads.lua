local S = {}

local shared = require("core.payloads.shared")
local board = require("core.payloads.board")
local predicates = require("core.payloads.predicates")
local builders = require("core.payloads.builders")

function S.install(M)
  local context = shared.install(M)
  board.install(M, context)
  predicates.install(M, context)
  builders.install(M, context)
end

return S
