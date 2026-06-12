local S = {}
local registry = require("core.registry")

function S.install(M)
  registry.load_indexed_installers("core.validators.index", M)
end

return S
