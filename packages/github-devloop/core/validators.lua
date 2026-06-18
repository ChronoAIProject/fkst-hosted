local S = {}
local registry = require("std.registry")

function S.install(M)
  registry.load_indexed_installers("core.validators.index", M, "github-devloop")
end

return S
