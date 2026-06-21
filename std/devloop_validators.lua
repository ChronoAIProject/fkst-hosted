local S = {}
local registry = require("std.registry")

function S.install(M)
  registry.load_indexed_installers("std.devloop_validators.index", M, M.restart_package_name or "github-devloop")
end

return S
