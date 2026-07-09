local github_adapter = require("forge.github")
local github_author_policy = require("devloop.github_author_policy")

local M = {}

local production_handle = nil

function M.github_options(exec)
  return github_author_policy.github_options(exec)
end

function M.new(exec, env_exec)
  if type(exec) ~= "function" then
    error("github-devloop: GitHub adapter requires an exec function")
  end
  return github_adapter.new(exec, M.github_options(env_exec))
end

function M.production_handle()
  if production_handle == nil then
    if type(exec_argv) ~= "function" then
      error("github-devloop: GitHub adapter requires exec_argv")
    end
    if type(exec_sync) ~= "function" then
      error("github-devloop: GitHub adapter requires exec_sync for author policy")
    end
    production_handle = M.new(exec_argv, exec_sync)
  end
  return production_handle
end

return M
