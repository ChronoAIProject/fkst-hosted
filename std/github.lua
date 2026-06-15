local exec_wrap = require("std.github.exec")

local M = {}

function M.new(exec)
  assert(type(exec) == "function", "std.github.new requires an exec function")
  local handle = {}
  function handle._exec(cmd, timeout, context)
    return exec_wrap.run(exec, cmd, timeout, context)
  end
  require("std.github.issue").install(handle)
  return handle
end

return M
