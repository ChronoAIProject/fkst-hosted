local exec_wrap = require("std.git.exec")

local M = {}

function M.new(exec)
  assert(type(exec) == "function", "std.git.new requires an exec function")
  local handle = {}
  function handle._exec(argv, timeout, context)
    return exec_wrap.run(exec, argv, timeout, context)
  end
  return handle
end

return M
