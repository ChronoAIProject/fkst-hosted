local env = require("workflow.env")

local M = {}

local function command_reader(allowed_env)
  return function(name)
    if not allowed_env[name] then
      error("archaudit: invalid-env-name: env name is not allowed")
    end
    return 'printf %s "$' .. name .. '"'
  end
end

function M.read_env(allowed_env, options)
  return env.read_env(command_reader(allowed_env), options)
end

return M
