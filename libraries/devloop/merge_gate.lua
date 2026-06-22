local S = {}
local shared = require("devloop.merge_gate.shared")
local ci_gate = require("devloop.merge_gate.ci_gate")
local self_heal = require("devloop.merge_gate.self_heal")
local verified_merge = require("devloop.merge_gate.verified_merge")

function S.install(M)
local shared_helpers = shared.install(M)
ci_gate.install(M, shared_helpers)
self_heal.install(M, shared_helpers)
verified_merge.install(M, shared_helpers)
end

return S
