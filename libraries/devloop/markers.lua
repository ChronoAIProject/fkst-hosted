local S = {}

local builders = require("devloop.markers.builders")
local shared = require("devloop.markers.shared")

function S.install(M)
M.normalize_intake_service_class = shared.normalize_intake_service_class
M.is_intake_service_class = shared.is_intake_service_class
builders.install(M, shared)
end

return S
