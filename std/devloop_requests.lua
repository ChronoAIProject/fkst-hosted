local S = {}

local labels = require("std.devloop_requests.labels")
local lifecycle = require("std.devloop_requests.lifecycle")
local review = require("std.devloop_requests.review")
local bodies = require("std.devloop_requests.bodies")

function S.install(M)
local shared = require("std.devloop_requests.shared").new(M)
labels.install(M, shared)
lifecycle.install(M, shared)
review.install(M, shared)
bodies.install(M, shared)
end

return S
