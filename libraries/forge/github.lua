local exec_wrap = require("forge.github.exec")
local result = require("forge.github.result")

local M = {}
local production_handles = {}

M.gh_result = result.gh_result

function M.new(exec, opts)
  assert(type(exec) == "function", "forge.github.new requires an exec function")
  local options = opts or {}
  local handle = {}
  handle._trusted_author_policy = options.trusted_author_policy
  function handle._exec(argv, timeout, context, stdout_policy)
    return exec_wrap.run(exec, argv, timeout, context, stdout_policy, handle._trusted_author_policy)
  end
  require("forge.github.issue").install(handle)
  require("forge.github.entities").install(handle)
  require("forge.github.comments").install(handle)
  require("forge.github.graphql").install(handle)
  require("forge.github.workflows").install(handle)
  return handle
end

function M.production_handle(owner, opts)
  local key = tostring(owner or "forge.github")
  if opts ~= nil then
    key = key .. ":" .. tostring(opts.trusted_author_policy)
  end
  local handle = production_handles[key]
  if handle == nil then
    if type(exec_argv) ~= "function" then
      error(key .. ": GitHub adapter requires exec_argv")
    end
    handle = M.new(exec_argv, opts)
    production_handles[key] = handle
  end
  return handle
end

return M
