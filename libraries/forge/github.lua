local exec_wrap = require("forge.github.exec")
local result = require("forge.github.result")

local M = {}

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

return M
