local S = {}

local builders = require("core.markers.builders")
local facts = require("core.markers.facts")
local shared = require("core.markers.shared")

function S.install(M)
builders.install(M, shared)
facts.install(M, shared)
end

return S
