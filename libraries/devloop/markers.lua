local S = {}

local builders = require("devloop.markers.builders")
local facts = require("devloop.markers.facts")
local shared = require("devloop.markers.shared")

function S.install(M)
builders.install(M, shared)
facts.install(M, shared)
end

return S
