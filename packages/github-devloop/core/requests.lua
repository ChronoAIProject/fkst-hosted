local S = {}

local labels = require("core.requests.labels")
local lifecycle = require("core.requests.lifecycle")
local review = require("core.requests.review")
local bodies = require("core.requests.bodies")

function S.install(M)
local shared = require("core.requests.shared").new(M)
labels.install(M, shared)
lifecycle.install(M, shared)
review.install(M, shared)
bodies.install(M, shared)
end

return S
