local S = {}

local builders = require("std.devloop_markers.builders")
local facts = require("std.devloop_markers.facts")
local shared = require("std.devloop_markers.shared")

function S.install(M)
builders.install(M, shared)
facts.install(M, shared)
end

return S
