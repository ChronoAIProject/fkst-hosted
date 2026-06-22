local S = {}

local labels = require("devloop.requests.labels")
local lifecycle = require("devloop.requests.lifecycle")
local review = require("devloop.requests.review")
local bodies = require("devloop.requests.bodies")

function S.install(M)
local shared = require("devloop.requests.shared").new(M)
labels.install(M, shared)
lifecycle.install(M, shared)
review.install(M, shared)
bodies.install(M, shared)
end

return S
