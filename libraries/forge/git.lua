local exec_wrap = require("forge.git.exec")

local M = {}
local production_handles = {}

local function require_scope_root(root)
  local value = tostring(root or ""):gsub("/+$", "")
  if value == "" or value:sub(1, 1) ~= "/" or value:find("[\r\n%z]") ~= nil then
    error("forge.git.scoped requires an absolute checkout root")
  end
  return value
end

function M.new(exec)
  assert(type(exec) == "function", "forge.git.new requires an exec function")
  local handle = {}
  function handle._exec(argv, timeout, context)
    return exec_wrap.run(exec, argv, timeout, context)
  end
  require("forge.git.refs").install(handle)
  return handle
end

-- Bind every repository-level operation to one verified checkout. Worktree-level
-- operations may add a second `-C`; Git applies the last one and still starts from
-- the granted repository rather than the process working directory.
function M.scoped(exec, root)
  assert(type(exec) == "function", "forge.git.scoped requires an exec function")
  local scope_root = require_scope_root(root)
  local handle = M.new(function(opts)
    local argv = opts and opts.argv
    if type(argv) ~= "table" or argv[1] ~= "git" then
      error("forge.git.scoped received non-git argv")
    end
    local scoped_argv = { "git", "-C", scope_root }
    for index = 2, #argv do
      table.insert(scoped_argv, argv[index])
    end
    return exec({ argv = scoped_argv, timeout = opts.timeout })
  end)
  handle.scope_root = scope_root
  return handle
end

function M.production_handle(owner)
  local key = tostring(owner or "forge.git")
  local handle = production_handles[key]
  if handle == nil then
    if type(exec_argv) ~= "function" then
      error(key .. ": git adapter requires exec_argv")
    end
    handle = M.new(exec_argv)
    production_handles[key] = handle
  end
  return handle
end

return M
