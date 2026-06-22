local S = {}

local shared = require("devloop.payloads.shared")
local board = require("devloop.payloads.board")
local predicates = require("devloop.payloads.predicates")
local builders = require("devloop.payloads.builders")

function S.install(M)
  local context = shared.install(M)
  board.install(M, context)
  predicates.install(M, context)
  builders.install(M, context)
end

return S
