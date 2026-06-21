local S = {}
local shared = require("std.devloop_merge_gate.shared")
local ci_gate = require("std.devloop_merge_gate.ci_gate")
local self_heal = require("std.devloop_merge_gate.self_heal")
local verified_merge = require("std.devloop_merge_gate.verified_merge")

function S.install(M)
local shared_helpers = shared.install(M)
ci_gate.install(M, shared_helpers)
self_heal.install(M, shared_helpers)
verified_merge.install(M, shared_helpers)
end

return S
