local exec_wrap = require("std.github.exec")

local M = {}

function M.new(exec)
  assert(type(exec) == "function", "std.github.new requires an exec function")
  local handle = {}
  function handle._exec(argv, timeout, context)
    return exec_wrap.run(exec, argv, timeout, context)
  end
  require("std.github.issue").install(handle)
  require("std.github.entities").install(handle)
  return handle
end

return M
