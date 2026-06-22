local exec_wrap = require("forge.git.exec")

local M = {}

function M.new(exec)
  assert(type(exec) == "function", "forge.git.new requires an exec function")
  local handle = {}
  function handle._exec(argv, timeout, context)
    return exec_wrap.run(exec, argv, timeout, context)
  end
  require("forge.git.refs").install(handle)
  return handle
end

return M
